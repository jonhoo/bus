use std::sync::atomic;
use std::sync::mpsc;
use std::thread;

use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::Arc;

// TODO: blocking reads
// TODO: spin before parking (https://github.com/Amanieu/parking_lot/issues/5)
// TODO: notify readers when writer is dropped

struct Seat<T: Clone> {
    read: atomic::AtomicUsize,
    max: usize,
    val: Option<T>,
    waiting: Option<thread::Thread>,
}

impl<T: Clone> Seat<T> {
    fn take(&self) -> T {
        let read = self.read.load(atomic::Ordering::Acquire);

        // NOTE
        // we must extract the value *before* we decrement the number of remaining items otherwise,
        // the object might be replaced by the time we read it!
        let v = self.val.clone().expect("");

        // let writer know that we no longer need this item
        self.read.fetch_add(1, atomic::Ordering::AcqRel);

        if read + 1 == self.max {
            if let Some(ref t) = self.waiting {
                // writer was waiting for us to finish with this
                t.unpark();
            }
            // no-one else will be cloning this value, so we could just take it instead of cloning
            // it. unfortunately, this requires having a mutable reference to the option, so we
            // don't do it for now.
        }

        return v;
    }
}

impl<T: Clone> Default for Seat<T> {
    fn default() -> Self {
        Seat {
            read: atomic::AtomicUsize::new(0),
            waiting: None,
            max: 0,
            val: None,
        }
    }
}

/// BusInner encapsulates data that both the writer and the readers need to access.
struct BusInner<T: Clone> {
    ring: Vec<Seat<T>>,
    len: usize,
    tail: atomic::AtomicUsize,
    leaving: mpsc::Sender<usize>,
}

// We need Sync for BusInner to share it in an Arc.
// This is safe here as long as there is only ever one mutable reference (specifically, the one
// held by the writer). Concurrent reads are fine as long as they follow the reader protocol (which
// BusReader does).
struct UnsafeBusInner<T: Clone>(UnsafeCell<BusInner<T>>);
unsafe impl<T: Clone> Sync for UnsafeBusInner<T> {}

// Make the indirection through Arc<UnsafeCell> a bit nicer to work with
impl<T: Clone> Deref for UnsafeBusInner<T> {
    type Target = UnsafeCell<BusInner<T>>;
    fn deref(&self) -> &Self::Target {
        return &self.0;
    }
}
type Inner<T: Clone> = Arc<UnsafeBusInner<T>>;

pub struct Bus<T: Clone> {
    state: Inner<T>,
    readers: usize,

    // rleft keeps track of readers that should be skipped for each index. we must do this because
    // .read will be < max for those indices, even though all active readers have received them.
    rleft: Vec<usize>,

    // leaving is used by receivers to signal that they are done
    leaving: mpsc::Receiver<usize>,
}

impl<T: Clone> Bus<T> {
    pub fn new(mut len: usize) -> Bus<T> {
        use std::iter;

        // ring buffer must have room for one padding element
        len += 1;

        let (leave_tx, leave_rx) = mpsc::channel();

        let inner = Arc::new(UnsafeBusInner(UnsafeCell::new(BusInner {
            ring: (0..len).map(|_| Seat::default()).collect(),
            tail: atomic::AtomicUsize::new(0),
            leaving: leave_tx,
            len: len,
        })));

        Bus {
            state: inner,
            readers: 0,
            rleft: iter::repeat(0).take(len).collect(),
            leaving: leave_rx,
        }
    }

    #[inline(always)]
    fn inner(&mut self) -> &mut BusInner<T> {
        // TODO: document why this is safe
        unsafe { &mut *self.state.get() }
    }

    fn broadcast_inner(&mut self, val: T, block: bool) -> Result<(), T> {
        let tail = self.inner().tail.load(atomic::Ordering::Relaxed);

        // we want to check if the next element over is free to ensure that we always leave one
        // empty space between the head and the tail. This is necessary so that readers can
        // distinguish between an empty and a full list. If the fence seat is free, the seat at
        // tail must also be free, which is simple enough to show by induction (exercise for the
        // reader).
        let fence = (tail + 1) % self.inner().len;

        // scope mutable borrow of self
        let fence_read = self.inner().ring[fence].read.load(atomic::Ordering::Acquire);
        if fence_read == self.inner().ring[fence].max - self.rleft[fence] {
            // next one over is also free, we have a free seat!
            let readers = self.readers;
            {
                let next = &mut self.inner().ring[tail];
                next.max = readers;
                next.val = Some(val);
                next.waiting = None;
                next.read.store(0, atomic::Ordering::Release);
            }
            self.rleft[tail] = 0;
            // now tell readers that they can read
            let len = self.inner().len;
            self.inner().tail.store((tail + 1) % len, atomic::Ordering::Release);
            return Ok(());
        }

        // there's no room left, so we check if any readers have left, which might increment
        // self.rleft[tail].
        while let Ok(mut left) = self.leaving.try_recv() {
            // a reader has left! this means that every seat between `left` and `tail-1` has max
            // set one too high. we track the number of such "missing" reads that should be ignored
            // in self.rleft, and compensate for them when looking at seat.read above.
            while left != tail {
                self.rleft[left] += 1;
                left = (left + 1) % self.inner().len
            }
        }

        if fence_read == self.inner().ring[fence].max - self.rleft[fence] {
            // the next block is now free!
            self.broadcast_inner(val, block)
        } else if block {
            use std::time::Duration;

            // park, wait to be unparked, and retry
            // we need the atomics to ensure reader threads will see the new .waiting
            self.inner().ring[fence].read.load(atomic::Ordering::Acquire);
            self.inner().ring[fence].waiting = Some(thread::current());
            self.inner().ring[fence].read.fetch_add(0, atomic::Ordering::Release);
            thread::park_timeout(Duration::new(0, 1000));
            self.broadcast_inner(val, block)
        } else {
            Err(val)
        }
    }

    pub fn try_broadcast(&mut self, val: T) -> Result<(), T> {
        self.broadcast_inner(val, false)
    }

    pub fn broadcast(&mut self, val: T) {
        if let Err(..) = self.broadcast_inner(val, true) {
            unreachable!("blocking broadcast_inner can't fail");
        }
    }

    pub fn add_rx(&mut self) -> BusReader<T> {
        self.readers += 1;

        BusReader {
            bus: self.state.clone(),
            head: self.inner().tail.load(atomic::Ordering::Relaxed),
            leaving: self.inner().leaving.clone(),
        }
    }
}

pub struct BusReader<T: Clone> {
    bus: Inner<T>,
    head: usize,
    leaving: mpsc::Sender<usize>,
}

unsafe impl<T: Clone> Send for BusReader<T> {}

impl<T: Clone> BusReader<T> {
    fn inner(&mut self) -> &BusInner<T> {
        // safe because we only acccess:
        //  - .tail, which is atomic
        //  - .ring.take, which is safe for concurrent read/write, since it effectively implements
        //    a RwLock on each entry.
        unsafe { &*self.bus.get() }
    }

    pub fn recv(&mut self) -> Result<T, ()> {
        let tail = self.inner().tail.load(atomic::Ordering::Acquire);
        if tail == self.head {
            // buffer is empty
            return Err(());
        }

        let head = self.head;
        let ret = self.inner().ring[head].take();
        self.head = (head + 1) % self.inner().len;
        Ok(ret)
    }
}

impl<T: Clone> Drop for BusReader<T> {
    #[allow(unused_must_use)]
    fn drop(&mut self) {
        // we allow not checking the result here because the writer might have gone away, which
        // would result in an error, but is okay nonetheless.
        self.leaving.send(self.head);
    }
}
