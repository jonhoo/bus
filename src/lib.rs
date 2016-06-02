extern crate atomic_option;
use atomic_option::AtomicOption;

use std::sync::atomic;
use std::sync::mpsc;
use std::thread;

use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::Arc;

// TODO: blocking reads
// TODO: spin before parking (https://github.com/Amanieu/parking_lot/issues/5)
// TODO: notify readers when writer is dropped

struct SeatState<T: Clone> {
    max: usize,
    val: Option<T>,
}

struct MutSeatState<T: Clone>(UnsafeCell<SeatState<T>>);
unsafe impl<T: Clone> Sync for MutSeatState<T> {}
impl<T: Clone> Deref for MutSeatState<T> {
    type Target = UnsafeCell<SeatState<T>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

struct Seat<T: Clone> {
    read: atomic::AtomicUsize,
    state: MutSeatState<T>,

    // is the writer waiting for this seat to be emptied? needs to be atomic since both the last
    // reader and the writer might be accessing it at the same time.
    waiting: AtomicOption<thread::Thread>,
}

impl<T: Clone> Seat<T> {
    fn take(&self) -> T {
        let read = self.read.load(atomic::Ordering::Acquire);

        // the writer will only modify this element when .read hits .max - writer.rleft[i]. we can
        // be sure that this is not currently the case (which means it's safe for us to read)
        // because:
        //
        //  - .max is set to the number of readers at the time when the write happens
        //  - any joining readers will start at a later seat
        //  - so, at most .max readers will call .take() on this seat this time around the buffer
        //  - a reader must leave either *before* or *after* a call to recv. there are two cases:
        //
        //    - it leaves before, rleft is decremented, but .take is not called
        //    - it leaves after, .take is called, but head has been incremented, so rleft will be
        //      decremented for the *next* seat, not this one
        //
        //    so, either .take is called, and .read is incremented, or writer.rleft is incremented.
        //    thus, for a writer to modify this element, *all* readers at the time of the previous
        //    write to this seat must have either called .take or have left.
        //  - since we are one of those readers, this cannot be true, so it's safe for us to assume
        //    that there is no concurrent writer for this seat
        let state = unsafe { &*self.state.get() };
        assert!(read < state.max,
                "reader hit seat with exhausted reader count");

        let mut waiting = None;

        // NOTE
        // we must extract the value *before* we decrement the number of remaining items otherwise,
        // the object might be replaced by the time we read it!
        let v = if read + 1 == state.max {
            // we're the last reader, so we may need to notify the writer there's space in the buf.
            // can be relaxed, since the acquire at the top already guarantees that we'll see
            // updates.
            waiting = self.waiting.take(atomic::Ordering::Relaxed);

            // since we're the last reader, no-one else will be cloning this value, so we can
            // safely take a mutable reference, and just take the val instead of cloning it.
            unsafe { &mut *self.state.get() }.val.take().unwrap()
        } else {
            state.val.clone().expect("seat that should be occupied was empty")
        };

        // let writer know that we no longer need this item.
        // state is no longer safe to access.
        drop(state);
        self.read.fetch_add(1, atomic::Ordering::AcqRel);

        if let Some(t) = waiting {
            // writer was waiting for us to finish with this
            t.unpark();
        }

        return v;
    }
}

impl<T: Clone> Default for Seat<T> {
    fn default() -> Self {
        Seat {
            read: atomic::AtomicUsize::new(0),
            waiting: AtomicOption::empty(),
            state: MutSeatState(UnsafeCell::new(SeatState {
                max: 0,
                val: None,
            })),
        }
    }
}

/// BusInner encapsulates data that both the writer and the readers need to access.
struct BusInner<T: Clone> {
    ring: Vec<Seat<T>>,
    len: usize,
    tail: atomic::AtomicUsize,
}

pub struct Bus<T: Clone> {
    state: Arc<BusInner<T>>,
    readers: usize,

    // rleft keeps track of readers that should be skipped for each index. we must do this because
    // .read will be < max for those indices, even though all active readers have received them.
    rleft: Vec<usize>,

    // leaving is used by receivers to signal that they are done
    leaving: (mpsc::Sender<usize>, mpsc::Receiver<usize>),
}

impl<T: Clone> Bus<T> {
    pub fn new(mut len: usize) -> Bus<T> {
        use std::iter;

        // ring buffer must have room for one padding element
        len += 1;

        let inner = Arc::new(BusInner {
            ring: (0..len).map(|_| Seat::default()).collect(),
            tail: atomic::AtomicUsize::new(0),
            len: len,
        });

        Bus {
            state: inner,
            readers: 0,
            rleft: iter::repeat(0).take(len).collect(),
            leaving: mpsc::channel(),
        }
    }

    fn broadcast_inner(&mut self, val: T, block: bool) -> Result<(), T> {
        let tail = self.state.tail.load(atomic::Ordering::Relaxed);

        // we want to check if the next element over is free to ensure that we always leave one
        // empty space between the head and the tail. This is necessary so that readers can
        // distinguish between an empty and a full list. If the fence seat is free, the seat at
        // tail must also be free, which is simple enough to show by induction (exercise for the
        // reader).
        let fence = (tail + 1) % self.state.len;
        let fence_read = self.state.ring[fence].read.load(atomic::Ordering::Acquire);

        // unsafe here is safe because we are the only possible writer, and since we're not
        // writing, reading must be safe
        if fence_read == unsafe { &*self.state.ring[fence].state.get() }.max - self.rleft[fence] {
            // next one over is free, we have a free seat!
            let readers = self.readers;
            {
                let next = &self.state.ring[tail];
                // we are the only writer, so no-one else can be writing. however, since we're
                // mutating state, we also need for there to be no readers for this to be safe. the
                // argument for why this is the case is roughly an inverse of the argument for why
                // the unsafe block in Seat.take() is safe.  basically, since
                //
                //   .read + .rleft == .max
                //
                // we know all readers at the time of the seat's previous write have accessed this
                // seat. we also know that no other readers will access that seat (they must have
                // started at later seats). thus, we are the only thread accessing this seat, and
                // so we can safely access it as mutable.
                let state = unsafe { &mut *next.state.get() };
                state.max = readers;
                state.val = Some(val);
                next.waiting.replace(None, atomic::Ordering::Relaxed);
                next.read.store(0, atomic::Ordering::Release);
            }
            self.rleft[tail] = 0;
            // now tell readers that they can read
            let len = self.state.len;
            self.state.tail.store((tail + 1) % len, atomic::Ordering::Release);
            return Ok(());
        }

        // there's no room left, so we check if any readers have left, which might increment
        // self.rleft[tail].
        while let Ok(mut left) = self.leaving.1.try_recv() {
            // a reader has left! this means that every seat between `left` and `tail-1` has max
            // set one too high. we track the number of such "missing" reads that should be ignored
            // in self.rleft, and compensate for them when looking at seat.read above.
            while left != tail {
                self.rleft[left] += 1;
                left = (left + 1) % self.state.len
            }
        }

        // we're not writing, so all refs are reads, so safe
        if fence_read == unsafe { &*self.state.ring[fence].state.get() }.max - self.rleft[fence] {
            // the next block is now free!
            self.broadcast_inner(val, block)
        } else if block {
            use std::time::Duration;

            // park and tell readers to notify on last read
            self.state.ring[fence]
                .waiting
                .replace(Some(Box::new(thread::current())), atomic::Ordering::Relaxed);

            // we need the atomic fetch_add to ensure reader threads will see the new .waiting
            self.state.ring[fence].read.fetch_add(0, atomic::Ordering::Release);

            // wait to be unparked, and retry
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
            head: self.state.tail.load(atomic::Ordering::Relaxed),
            leaving: self.leaving.0.clone(),
        }
    }
}

pub struct BusReader<T: Clone> {
    bus: Arc<BusInner<T>>,
    head: usize,
    leaving: mpsc::Sender<usize>,
}

impl<T: Clone> BusReader<T> {
    pub fn recv(&mut self) -> Result<T, ()> {
        let tail = self.bus.tail.load(atomic::Ordering::Acquire);
        if tail == self.head {
            // buffer is empty
            return Err(());
        }

        let head = self.head;
        let ret = self.bus.ring[head].take();

        // safe because len is read-only
        self.head = (head + 1) % self.bus.len;
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
