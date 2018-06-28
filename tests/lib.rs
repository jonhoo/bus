extern crate bus;
#[cfg(feature = "async")]
extern crate futures;
#[cfg(feature = "async")]
extern crate tokio;

use std::sync::mpsc;
use std::time;

#[test]
fn it_works() {
    let mut c = bus::Bus::new(10);
    let mut r1 = c.add_rx();
    let mut r2 = c.add_rx();
    assert_eq!(c.try_broadcast(true), Ok(()));
    assert_eq!(r1.try_recv(), Ok(true));
    assert_eq!(r2.try_recv(), Ok(true));
}

#[test]
fn it_fails_when_full() {
    let mut c = bus::Bus::new(1);
    let r1 = c.add_rx();
    assert_eq!(c.try_broadcast(true), Ok(()));
    assert_eq!(c.try_broadcast(false), Err(false));
    drop(r1);
}

#[test]
fn it_succeeds_when_not_full() {
    let mut c = bus::Bus::new(1);
    let mut r1 = c.add_rx();
    assert_eq!(c.try_broadcast(true), Ok(()));
    assert_eq!(c.try_broadcast(false), Err(false));
    assert_eq!(r1.try_recv(), Ok(true));
    assert_eq!(c.try_broadcast(true), Ok(()));
}

#[test]
fn it_fails_when_empty() {
    let mut c = bus::Bus::<bool>::new(10);
    let mut r1 = c.add_rx();
    assert_eq!(r1.try_recv(), Err(mpsc::TryRecvError::Empty));
}

#[test]
fn it_reads_when_full() {
    let mut c = bus::Bus::new(1);
    let mut r1 = c.add_rx();
    assert_eq!(c.try_broadcast(true), Ok(()));
    assert_eq!(r1.try_recv(), Ok(true));
}

#[test]
fn it_iterates() {
    use std::thread;

    let mut tx = bus::Bus::new(2);
    let mut rx = tx.add_rx();
    let j = thread::spawn(move || {
        for i in 0..1000 {
            tx.broadcast(i);
        }
    });

    let mut ii = 0;
    for i in rx.iter() {
        assert_eq!(i, ii);
        ii += 1;
    }

    j.join().unwrap();
    assert_eq!(ii, 1000);
    assert_eq!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected));
}

#[test]
fn it_detects_closure() {
    let mut tx = bus::Bus::new(1);
    let mut rx = tx.add_rx();
    assert_eq!(tx.try_broadcast(true), Ok(()));
    assert_eq!(rx.try_recv(), Ok(true));
    assert_eq!(rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    drop(tx);
    assert_eq!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected));
}

#[test]
fn it_recvs_after_close() {
    let mut tx = bus::Bus::new(1);
    let mut rx = tx.add_rx();
    assert_eq!(tx.try_broadcast(true), Ok(()));
    drop(tx);
    assert_eq!(rx.try_recv(), Ok(true));
    assert_eq!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected));
}

#[test]
fn it_handles_leaves() {
    let mut c = bus::Bus::new(1);
    let mut r1 = c.add_rx();
    let r2 = c.add_rx();
    assert_eq!(c.try_broadcast(true), Ok(()));
    drop(r2);
    assert_eq!(r1.try_recv(), Ok(true));
    assert_eq!(c.try_broadcast(true), Ok(()));
}

#[test]
fn it_runs_blocked_writes() {
    use std::thread;

    let mut c = Box::new(bus::Bus::new(1));
    let mut r1 = c.add_rx();
    c.broadcast(true); // this is fine

    // buffer is now full
    assert_eq!(c.try_broadcast(false), Err(false));
    // start other thread that blocks
    let c = thread::spawn(move || {
        c.broadcast(false);
    });
    // unblock sender by receiving
    assert_eq!(r1.try_recv(), Ok(true));
    // drop r1 to release other thread and safely drop c
    drop(r1);
    c.join().unwrap();
}

#[test]
fn it_runs_blocked_reads() {
    use std::thread;
    use std::sync::mpsc;

    let mut tx = Box::new(bus::Bus::new(1));
    let mut rx = tx.add_rx();
    // buffer is now empty
    assert_eq!(rx.try_recv(), Err(mpsc::TryRecvError::Empty));
    // start other thread that blocks
    let c = thread::spawn(move || {
        rx.recv().unwrap();
    });
    // unblock receiver by broadcasting
    tx.broadcast(true);
    // check that thread now finished
    c.join().unwrap();
}

#[test]
fn it_can_count_to_10000() {
    use std::thread;

    let mut c = bus::Bus::new(2);
    let mut r1 = c.add_rx();
    let j = thread::spawn(move || {
        for i in 0..10_000 {
            c.broadcast(i);
        }
    });

    for i in 0..10_000 {
        assert_eq!(r1.recv(), Ok(i));
    }

    j.join().unwrap();
    assert_eq!(r1.try_recv(), Err(mpsc::TryRecvError::Disconnected));
}

#[test]
fn test_busy() {
    use std::thread;

    // start a bus with limited space
    let mut bus = bus::Bus::new(1);

    // first receiver only receives 5 items
    let mut rx1 = bus.add_rx();
    let t1 = thread::spawn(move || {
        for _ in 0..5 {
            rx1.recv().unwrap();
        }
        drop(rx1);
    });

    // second receiver receives 10 items
    let mut rx2 = bus.add_rx();
    let t2 = thread::spawn(move || {
        for _ in 0..10 {
            rx2.recv().unwrap();
        }
        drop(rx2);
    });

    // let receivers start
    std::thread::sleep(time::Duration::from_millis(500));

    // try to send 25 items -- should work fine
    for i in 0..25 {
        std::thread::sleep(time::Duration::from_millis(100));
        match bus.try_broadcast(i) {
            Ok(_) => (),
            Err(e) => println!("Broadcast failed {}", e),
        }
    }

    // done sending -- wait for receivers (which should already be done)
    t1.join().unwrap();
    t2.join().unwrap();
    assert!(true);
}

fn spawn_deadlined<F>(deadline: time::Instant, future: F)
    where F: futures::Future + Send + 'static,
{
    use futures::Future;
    tokio::spawn({
        tokio::timer::Deadline::new(future, deadline)
            .map(|_| ())
            .map_err(|e| {
                panic!("\ndeadline failure\ndeadline elapsed: {:?}\ntimer error: {:?}\n",
                    e.is_elapsed(), e.into_timer());
            })
    });
}

#[test]
fn it_wakes_async_readers() {
    use futures::prelude::*;
    use futures::{future, stream};
    use std::sync;

    let items = sync::Arc::new(sync::Mutex::new(vec![]));
    let tokio_items = items.clone();

    tokio::run(future::lazy(move || {
        let items = tokio_items;
        let started_at = time::Instant::now();
        let deadline = started_at + time::Duration::from_millis(2000);
        let mut bus = bus::Bus::new_async(1);

        {
            let mut spawn_eager = |nr| spawn_deadlined(deadline, {
                let items = items.clone();
                bus.add_rx()
                    .for_each(move |i| {
                        items.lock().unwrap().push((nr, i, started_at.elapsed()));
                        Ok(())
                    })
                    .map_err(|_| unreachable!())
            });

            spawn_eager(1);
            spawn_eager(2);
            spawn_eager(3);
        }

        spawn_deadlined(deadline, {
            stream::iter_ok(0..5)
                .and_then(|i| {
                    tokio::timer::Delay::new(time::Instant::now() + time::Duration::from_millis(100))
                        .map(move |()| i)
                        .map_err(|_: tokio::timer::Error| panic!("delay failed"))
                })
                .forward(bus.sink_map_err(|_| unreachable!()))
        });

        Ok(())
    }));

    let mut items = sync::Arc::try_unwrap(items).unwrap()
        .into_inner().unwrap();
    items.sort();
    assert_eq!(items, vec![]);
}

#[test]
fn it_wakes_async_readers_with_writer_spawned_first() {
    use futures::prelude::*;
    use futures::{future, stream};
    use std::sync;

    let items = sync::Arc::new(sync::Mutex::new(vec![]));
    let tokio_items = items.clone();

    tokio::run(future::lazy(move || {
        let items = tokio_items;
        let started_at = time::Instant::now();
        let deadline = started_at + time::Duration::from_millis(2000);
        let mut bus = bus::Bus::new_async(1);

        let intervals = vec![
            (bus.add_rx(), 1),
            (bus.add_rx(), 2),
            (bus.add_rx(), 3),
        ];

        spawn_deadlined(deadline, {
            stream::iter_ok(0..5)
                .and_then(|i| {
                    tokio::timer::Delay::new(time::Instant::now() + time::Duration::from_millis(100))
                        .map(move |()| i)
                        .map_err(|_: tokio::timer::Error| panic!("delay failed"))
                })
                .forward(bus.sink_map_err(|_| unreachable!()))
        });

        for (rx, nr) in intervals {
            spawn_deadlined(deadline, {
                let items = items.clone();
                rx
                    .for_each(move |i| {
                        items.lock().unwrap().push((nr, i, started_at.elapsed()));
                        Ok(())
                    })
                    .map_err(|_| unreachable!())
            });
        }

        Ok(())
    }));

    let mut items = sync::Arc::try_unwrap(items).unwrap()
        .into_inner().unwrap();
    items.sort();
    assert_eq!(items, vec![]);
}

#[test]
fn it_wakes_async_writers() {
    use futures::prelude::*;
    use futures::{future, stream};
    use std::sync;

    let items = sync::Arc::new(sync::Mutex::new(vec![]));
    let tokio_items = items.clone();

    tokio::run(future::lazy(move || {
        let items = tokio_items;
        let started_at = time::Instant::now();
        let deadline = started_at + time::Duration::from_millis(2000);
        let mut bus = bus::Bus::new_async(1);

        {
            let mut spawn_interval = |nr, millis| spawn_deadlined(deadline, {
                let items = items.clone();
                bus.add_rx()
                    .for_each(move |i| {
                        items.lock().unwrap().push((nr, i, started_at.elapsed()));
                        tokio::timer::Delay::new(time::Instant::now() + time::Duration::from_millis(millis))
                            .map_err(|_: tokio::timer::Error| panic!("delay failed"))
                    })
                    .map_err(|_| unreachable!())
            });

            spawn_interval(1, 25);
            spawn_interval(2, 75);
            spawn_interval(3, 125);
        }

        spawn_deadlined(deadline, {
            stream::iter_ok::<_, ()>(0..5)
                .forward(bus.sink_map_err(|_| unreachable!()))
        });

        Ok(())
    }));

    let mut items = sync::Arc::try_unwrap(items).unwrap()
        .into_inner().unwrap();
    items.sort();
    assert_eq!(items, vec![]);
}

#[test]
fn it_wakes_async_writers_with_writer_spawned_first() {
    use futures::prelude::*;
    use futures::{future, stream};
    use std::sync;

    let items = sync::Arc::new(sync::Mutex::new(vec![]));
    let tokio_items = items.clone();

    tokio::run(future::lazy(move || {
        let items = tokio_items;
        let started_at = time::Instant::now();
        let deadline = started_at + time::Duration::from_millis(2000);
        let mut bus = bus::Bus::new_async(1);

        let intervals = vec![
            (bus.add_rx(), 1, 25),
            (bus.add_rx(), 2, 75),
            (bus.add_rx(), 3, 125),
        ];

        spawn_deadlined(deadline, {
            stream::iter_ok::<_, ()>(0..5)
                .forward(bus.sink_map_err(|_| unreachable!()))
        });

        for (rx, nr, millis) in intervals {
            spawn_deadlined(deadline, {
                let items = items.clone();
                rx
                    .for_each(move |i| {
                        items.lock().unwrap().push((nr, i, started_at.elapsed()));
                        tokio::timer::Delay::new(time::Instant::now() + time::Duration::from_millis(millis))
                            .map_err(|_: tokio::timer::Error| panic!("delay failed"))
                    })
                    .map_err(|_| unreachable!())
            });
        }

        Ok(())
    }));

    let mut items = sync::Arc::try_unwrap(items).unwrap()
        .into_inner().unwrap();
    items.sort();
    assert_eq!(items, vec![]);
}
