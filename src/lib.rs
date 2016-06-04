//! Bus provides a lock-free, bounded, single-producer, multi-consumer, broadcast channel.
//!
//! It uses a circular buffer and atomic instructions to implement a lock-free single-producer,
//! multi-consumer channel. The interface is similar to that of the `std::sync::mpsc` channels,
//! except that multiple consumers (readers of the channel) can be produced, whereas only a single
//! sender can exist. Furthermore, in contrast to most multi-consumer FIFO queues, bus is
//! *broadcast*; every send goes to every consumer.
//!
//! I haven't seen this particular implementation in literature (some extra bookkeeping is
//! necessary to allow multiple consumers), but a lot of related reading can be found in Ross
//! Bencina's blog post ["Some notes on lock-free and wait-free
//! algorithms"](http://www.rossbencina.com/code/lockfree).
//!
//! Bus achieves broadcast by cloning the element in question, which is why `T` must implement
//! `Clone`. However, Bus is clever about only cloning when necessary. Specifically, the last
//! consumer to see a given value will move it instead of cloning, which means no cloning is
//! happening for the single-consumer case. For cases where cloning is expensive, `Arc` should be
//! used instead.
//!
//! In a single-producer, single-consumer setup (which is the only one that Bus and
//! `mpsc::sync_channel` both support), Bus gets ~3x the performance of `mpsc::sync_channel` on
//! my machine. YMMV. You can check your performance on Nightly using
//!
//! ```console
//! $ cargo bench --features bench
//! ```
//!
//! To see multi-consumer results, run the benchmark utility instead (should work on stable too)
//!
//! ```console
//! $ cargo build --bin bench --release
//! $ target/release/bench
//! ```
//!
//! # Examples
//!
//! Single-send, multi-consumer example
//!
//! ```rust
//! use bus::Bus;
//! let mut bus = Bus::new(10);
//! let mut rx1 = bus.add_rx();
//! let mut rx2 = bus.add_rx();
//!
//! bus.broadcast("Hello");
//! assert_eq!(rx1.recv(), Ok("Hello"));
//! assert_eq!(rx2.recv(), Ok("Hello"));
//! ```
//!
//! Multi-send, multi-consumer example
//!
//! ```rust
//! use bus::Bus;
//! use std::thread;
//!
//! let mut bus = Bus::new(10);
//! let mut rx1 = bus.add_rx();
//! let mut rx2 = bus.add_rx();
//!
//! // start a thread that sends 1..100
//! let j = thread::spawn(move || {
//!     for i in 1..100 {
//!         bus.broadcast(i);
//!     }
//! });
//!
//! // every value should be received by both receivers
//! for i in 1..100 {
//!     // rx1
//!     assert_eq!(rx1.recv(), Ok(i));
//!     // and rx2
//!     assert_eq!(rx2.recv(), Ok(i));
//! }
//!
//! j.join().unwrap();
//! ```
//!
//! Many-to-many channel using a dispatcher
//!
//! ```rust
//! use bus::Bus;
//!
//! use std::thread;
//! use std::sync::mpsc;
//!
//! // set up fan-in
//! let (tx1, mix_rx) = mpsc::sync_channel(100);
//! let tx2 = tx1.clone();
//! // set up fan-out
//! let mut mix_tx = Bus::new(100);
//! let mut rx1 = mix_tx.add_rx();
//! let mut rx2 = mix_tx.add_rx();
//! // start dispatcher
//! thread::spawn(move || {
//!     for m in mix_rx.iter() {
//!         mix_tx.broadcast(m);
//!     }
//! });
//!
//! // sends on tx1 are received ...
//! tx1.send("Hello").unwrap();
//!
//! // ... by both receiver rx1 ...
//! assert_eq!(rx1.recv(), Ok("Hello"));
//! // ... and receiver rx2
//! assert_eq!(rx2.recv(), Ok("Hello"));
//!
//! // same with sends on tx2
//! tx2.send("world").unwrap();
//! assert_eq!(rx1.recv(), Ok("world"));
//! assert_eq!(rx2.recv(), Ok("world"));
//! ```

#![cfg_attr(feature = "bench", feature(test))]

extern crate atomic_option;
use atomic_option::AtomicOption;

#[cfg(feature = "bench")]
extern crate test;

use std::sync::atomic;
use std::sync::mpsc;
use std::thread;

use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::Arc;

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

/// A Seat is a single location in the circular buffer.
/// Each Seat knows how many readers are expected to access it, as well as how many have. The
/// producer will never modify a seat's state unless all readers for a particular seat have either
/// called `.take()` on it, or have left (see `Bus.rleft`).
///
/// The producer walks the seats of the ring in order, and will always only modify the seat at
/// `tail + 1` once all readers have finished with the seat at `head + 2`. A reader will never
/// access a seat unless it is between the reader's `head` and the producer's `tail`. Together,
/// these properties ensure that a Seat is either accessed only by readers, or by only the
/// producer.
///
/// The `read` attribute is used to ensure that readers see the most recent write to the seat when
/// they access it. This is done using `atomic::Ordering::Acquire` and `atomic::Ordering::Release`.
struct Seat<T: Clone> {
    read: atomic::AtomicUsize,
    state: MutSeatState<T>,

    // is the writer waiting for this seat to be emptied? needs to be atomic since both the last
    // reader and the writer might be accessing it at the same time.
    waiting: AtomicOption<thread::Thread>,
}

impl<T: Clone> Seat<T> {
    /// take is used by a reader to extract a copy of the value stored on this seat. only readers
    /// that were created strictly before the time this seat was last written to by the producer
    /// are allowed to call this method, and they may each only call it once.
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
/// The tail is only ever modified by the producer, and read by the consumers.
/// The length of the bus is instantiated when the bus is created, and is never modified.
struct BusInner<T: Clone> {
    ring: Vec<Seat<T>>,
    len: usize,
    tail: atomic::AtomicUsize,
}

/// Bus is the main interconnect for broadcast messages.
/// It can be used to send broadcast messages, or to connect additional consumers.
/// Note that there is **currently** no way for consumers to detect that the producer has left (and
/// that the bus is therefore closed). Bear this in mind when using Bus.
pub struct Bus<T: Clone> {
    state: Arc<BusInner<T>>,

    // current number of readers
    readers: usize,

    // rleft keeps track of readers that should be skipped for each index. we must do this because
    // .read will be < max for those indices, even though all active readers have received them.
    rleft: Vec<usize>,

    // leaving is used by receivers to signal that they are done
    leaving: (mpsc::Sender<usize>, mpsc::Receiver<usize>),

    // waiting is used by receivers to signal that they are waiting for new entries, and where they
    // are waiting
    waiting: (mpsc::Sender<(thread::Thread, usize)>, mpsc::Receiver<(thread::Thread, usize)>),
}

impl<T: Clone> Bus<T> {
    /// Allocates a new bus.
    ///
    /// The provided length should be sufficient to absorb temporary peaks in the data flow, and is
    /// thus workflow-dependent. Bus performance degrades somewhat when the queue is full, so it is
    /// generally better to set this high than low unless you are pressed for memory.
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
            waiting: mpsc::channel(),
        }
    }

    /// Get the expected number of reads for the given seat.
    /// This number will always be conservative, in that fewer reads may be fine. Specifically,
    /// `.rleft` may not be sufficiently up-to-date to account for all readers that have left.
    fn expected(&mut self, at: usize) -> usize {
        // since only the producer will modify the ring, and &mut self guarantees that *we* are the
        // producer, no-one is modifying the ring. Multiple read-only borrows are safe, and so the
        // cast below is safe.
        unsafe { &*self.state.ring[at].state.get() }.max - self.rleft[at]
    }

    /// Attempts to place the given value on the bus.
    ///
    /// If the bus is full, the behavior depends on `block`. If false, the value given is returned
    /// in an `Err()`. Otherwise, the current thread will be parked until there is space in the bus
    /// again, and the broadcast will be tried again until it succeeds.
    ///
    /// Note that broadcasts will succeed even if there are no consumers!
    fn broadcast_inner(&mut self, val: T, block: bool) -> Result<(), T> {
        let tail = self.state.tail.load(atomic::Ordering::Relaxed);

        // we want to check if the next element over is free to ensure that we always leave one
        // empty space between the head and the tail. This is necessary so that readers can
        // distinguish between an empty and a full list. If the fence seat is free, the seat at
        // tail must also be free, which is simple enough to show by induction (exercise for the
        // reader).
        let fence = (tail + 1) % self.state.len;
        let fence_read = self.state.ring[fence].read.load(atomic::Ordering::Acquire);

        if fence_read == self.expected(fence) {
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
            let tail = (tail + 1) % self.state.len;
            self.state.tail.store(tail, atomic::Ordering::Release);

            // unblock any blocked receivers
            let mut add_back = Vec::with_capacity(self.readers);
            while let Ok((t, at)) = self.waiting.1.try_recv() {
                // the only readers we can't unblock are those that have already absorbed the
                // broadcast we just made, since they are blocking on the *next* broadcast
                if at == tail {
                    add_back.push((t, at))
                } else {
                    t.unpark();
                }
            }
            for w in add_back {
                // fine to do here because it is guaranteed not to block
                self.waiting.0.send(w).unwrap();
            }

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

        // is the fence block now free?
        if fence_read == self.expected(fence) {
            // yes, so the next block is now free!
            self.broadcast_inner(val, block)
        } else if block {
            use std::time::Duration;

            // park and tell readers to notify on last read
            // TODO: spin before parking (https://github.com/Amanieu/parking_lot/issues/5)
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

    /// Attempt to broadcast the given value to all consumers, but do not wait if bus is full.
    ///
    /// If the bus is currently full, the given value is returned in an `Err()`.
    /// Note that, in contrast to regular channels, a bus is *not* considered closed if there are
    /// no consumers, and thus broadcasts will continue to succeed.
    ///
    /// ```rust
    /// use bus::Bus;
    /// let mut tx = Bus::new(1);
    /// let mut rx = tx.add_rx();
    /// assert_eq!(tx.try_broadcast("Hello"), Ok(()));
    /// assert_eq!(tx.try_broadcast("world"), Err("world"));
    /// ```
    pub fn try_broadcast(&mut self, val: T) -> Result<(), T> {
        self.broadcast_inner(val, false)
    }

    /// Broadcast the given value to all consumers, waiting until the bus is not full if necessary.
    ///
    /// Note that, in contrast to regular channels, a bus is *not* considered closed if there are
    /// no consumers, and thus broadcasts will continue to succeed.
    pub fn broadcast(&mut self, val: T) {
        if let Err(..) = self.broadcast_inner(val, true) {
            unreachable!("blocking broadcast_inner can't fail");
        }
    }

    /// Add a new consumer to this bus.
    ///
    /// The new consumer will receive all *future* broadcasts on this bus.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use bus::Bus;
    /// use std::sync::mpsc::TryRecvError;
    ///
    /// let mut bus = Bus::new(10);
    /// let mut rx1 = bus.add_rx();
    ///
    /// bus.broadcast("Hello");
    ///
    /// // consumer present during broadcast sees update
    /// assert_eq!(rx1.recv(), Ok("Hello"));
    ///
    /// // new consumer does *not* see broadcast
    /// let mut rx2 = bus.add_rx();
    /// assert_eq!(rx2.try_recv(), Err(TryRecvError::Empty));
    ///
    /// // both consumers see new broadcast
    /// bus.broadcast("world");
    /// assert_eq!(rx1.recv(), Ok("world"));
    /// assert_eq!(rx2.recv(), Ok("world"));
    /// ```
    pub fn add_rx(&mut self) -> BusReader<T> {
        self.readers += 1;

        BusReader {
            bus: self.state.clone(),
            head: self.state.tail.load(atomic::Ordering::Relaxed),
            leaving: self.leaving.0.clone(),
            waiting: self.waiting.0.clone(),
        }
    }
}

/// A BusReader is a single consumer of Bus broadcasts.
/// It will see every new value that is passed to `.broadcast()` (or successful calls to
/// `.try_broadcast()`) on the Bus that it was created from.
///
/// Dropping a BusReader is perfectly safe, and will unblock the writer if it was waiting for that
/// read to see a particular update.
///
/// ```rust
/// use bus::Bus;
/// let mut tx = Bus::new(1);
/// let mut r1 = tx.add_rx();
/// let r2 = tx.add_rx();
/// assert_eq!(tx.try_broadcast(true), Ok(()));
/// assert_eq!(r1.recv(), Ok(true));
///
/// // the bus does not have room for another broadcast
/// // since it knows r2 has not yet read the first broadcast
/// assert_eq!(tx.try_broadcast(true), Err(true));
///
/// // dropping r2 tells the producer that there is a free slot
/// // (i.e., it has been read by everyone)
/// drop(r2);
/// assert_eq!(tx.try_broadcast(true), Ok(()));
/// ```
pub struct BusReader<T: Clone> {
    bus: Arc<BusInner<T>>,
    head: usize,
    leaving: mpsc::Sender<usize>,
    waiting: mpsc::Sender<(thread::Thread, usize)>,
}

impl<T: Clone> BusReader<T> {
    /// Attempts to read a broadcast from the bus.
    ///
    /// If the bus is empty, the behavior depends on `block`. If false,
    /// `Err(mpsc::TryRecvError::Empty)` is returned. Otherwise, the current thread will be parked
    /// until there is another broadcast on the bus, at which point the receive will be performed.
    fn recv_inner(&mut self, block: bool) -> Result<T, mpsc::TryRecvError> {
        // TODO: notify readers when writer is dropped and bus is closed

        let tail = self.bus.tail.load(atomic::Ordering::Acquire);
        if tail == self.head {
            use std::time::Duration;

            // buffer is empty
            if !block {
                return Err(mpsc::TryRecvError::Empty);
            }

            // park and tell writer to notify on write
            // TODO: spin before parking (https://github.com/Amanieu/parking_lot/issues/5)
            if let Err(..) = self.waiting.send((thread::current(), self.head)) {
                // writer has gone away, but this is not a reliable way to check
                // in particular, we may also have missed updates
                unimplemented!();
            }
            thread::park_timeout(Duration::new(0, 1000));
            return self.recv_inner(block);
        }

        let head = self.head;
        let ret = self.bus.ring[head].take();

        // safe because len is read-only
        self.head = (head + 1) % self.bus.len;
        Ok(ret)
    }

    /// Attempts to return a pending broadcast on this receiver without blocking.
    ///
    /// This method will never block the caller in order to wait for data to become available.
    /// Instead, this will always return immediately with a possible option of pending data on the
    /// channel.
    ///
    /// There is **currently** no way to detect that the producer has left and the bus has been
    /// closed.
    ///
    /// This mehtod is useful for a flavor of "optimistic check" before deciding to block on a
    /// receiver.
    ///
    /// ```rust
    /// use bus::Bus;
    /// use std::thread;
    ///
    /// let mut tx = Bus::new(10);
    /// let mut rx = tx.add_rx();
    ///
    /// // spawn a thread that will broadcast at some point
    /// let j = thread::spawn(move || {
    ///     tx.broadcast(true);
    /// });
    ///
    /// loop {
    ///     match rx.try_recv() {
    ///         Ok(val) => {
    ///             assert_eq!(val, true);
    ///             break;
    ///         }
    ///         Err(..) => {
    ///             // maybe we can do other useful work here
    ///             // or we can just busy-loop
    ///             thread::yield_now()
    ///         },
    ///     }
    /// }
    ///
    /// j.join().unwrap();
    /// ```
    pub fn try_recv(&mut self) -> Result<T, mpsc::TryRecvError> {
        self.recv_inner(false)
    }

    /// Read another broadcast message from the bus, and block if none are available.
    ///
    /// This function will always block the current thread if there is no data available and it's
    /// possible for more broadcasts to be sent. Once a broadcast is sent on the corresponding Bus,
    /// then this receiver will wake up and return that message.
    ///
    /// Be aware that this method is **currently** much slower than polling try_recv when there are
    /// many receivers. This is because the producer must check if there are any waiting readers on
    /// every write. When there are many concurrent readers, this check becomes much more
    /// expensive, which slows down writes significantly.
    ///
    /// There is also no way (again, **currently**) to detect that the producer has left and the
    /// bus has been closed, so this method will either return Ok or block indefinitely. This will
    /// change in the future.
    pub fn recv(&mut self) -> Result<T, mpsc::RecvError> {
        if let Ok(val) = self.recv_inner(true) {
            Ok(val)
        } else {
            unreachable!("blocking recv_inner can't fail");
        }
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

#[cfg(feature = "bench")]
#[bench]
fn bench_bus_one_to_one(b: &mut test::Bencher) {
    let mut c = Bus::new(100);
    let mut rx = c.add_rx();
    let j = thread::spawn(move || {
        loop {
            match rx.recv() {
                Ok(exit) if exit => break,
                Err(..) => break,
                _ => (),
            }
        }
    });
    b.iter(|| c.broadcast(false));
    c.broadcast(true);
    j.join().unwrap();
}

#[cfg(feature = "bench")]
#[bench]
fn bench_syncch_one_to_one(b: &mut test::Bencher) {
    let (tx, rx) = mpsc::sync_channel(100);
    let j = thread::spawn(move || {
        loop {
            match rx.recv() {
                Ok(exit) if exit => break,
                Err(..) => break,
                _ => (),
            }
        }
    });
    b.iter(|| tx.send(false).unwrap());
    tx.send(true).unwrap();
    j.join().unwrap();
}
