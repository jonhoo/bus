extern crate bus;
extern crate futures;

use std::time;

use futures::stream::Stream;
use futures::Async;

struct Sink;
impl futures::task::Unpark for Sink {
    fn unpark(&self) {}
}

macro_rules! assert_poll {
    ($rx:ident, $val:expr) => {{
        use std::sync::Arc;
        match futures::task::spawn($rx).poll_future(Arc::new(Sink)) {
            Ok(Async::Ready((v, rest))) => {
                assert_eq!(v, $val);
                rest.into_future()
            },
            _ => unreachable!(),
        }
    }}
}

macro_rules! assert_nopoll {
    ($rx:ident) => {{
        use std::sync::Arc;
        let mut t = futures::task::spawn($rx);
        match t.poll_future(Arc::new(Sink)) {
            Ok(Async::NotReady) => t,
            Ok(Async::Ready((None, _))) => {
                assert!(false, "expected bus not to be ready, but was disconnected");
                unreachable!();
            },
            Ok(Async::Ready((Some(v), _))) => {
                assert!(false, "expected bus not to be ready, got {:?}", v);
                unreachable!();
            },
            _ => unreachable!(),
        }
    }}
}

#[test]
fn it_works() {
    let mut c = bus::Bus::new(10);
    let r1 = c.add_rx().into_future();
    let r2 = c.add_rx().into_future();
    assert_eq!(c.try_broadcast(true), Ok(()));
    drop(assert_poll!(r1, Some(true)));
    drop(assert_poll!(r2, Some(true)));
}

#[test]
fn it_fails_when_full() {
    let mut c = bus::Bus::new(1);
    let r1 = c.add_rx().into_future();
    assert_eq!(c.try_broadcast(true), Ok(()));
    assert_eq!(c.try_broadcast(false), Err(false));
    drop(r1);
}

#[test]
fn it_succeeds_when_not_full() {
    let mut c = bus::Bus::new(1);
    let r1 = c.add_rx().into_future();
    assert_eq!(c.try_broadcast(true), Ok(()));
    assert_eq!(c.try_broadcast(false), Err(false));
    drop(assert_poll!(r1, Some(true)));
    assert_eq!(c.try_broadcast(true), Ok(()));
}

#[test]
fn it_fails_when_empty() {
    let mut c = bus::Bus::<bool>::new(10);
    let r1 = c.add_rx().into_future();
    drop(assert_nopoll!(r1));
}

#[test]
fn it_reads_when_full() {
    let mut c = bus::Bus::new(1);
    let r1 = c.add_rx().into_future();
    assert_eq!(c.try_broadcast(true), Ok(()));
    drop(assert_poll!(r1, Some(true)));
}

#[test]
fn it_iterates() {
    use std::thread;

    let mut tx = bus::Bus::new(2);
    let rx = tx.add_rx();
    let j = thread::spawn(move || {
        for i in 0..1000 {
            tx.broadcast(i);
        }
    });

    let mut ii = 0;
    let rx = rx.wait();
    for i in rx {
        assert_eq!(i, Ok(ii));
        ii += 1;
    }

    j.join().unwrap();
    assert_eq!(ii, 1000);
}

#[test]
fn it_detects_closure() {
    use std::sync::Arc;

    let mut tx = bus::Bus::new(1);
    let rx = tx.add_rx().into_future();
    assert_eq!(tx.try_broadcast(true), Ok(()));
    let rx = assert_poll!(rx, Some(true));
    let mut rx = assert_nopoll!(rx);
    drop(tx);
    match rx.poll_future(Arc::new(Sink)) {
        Ok(Async::Ready((v, rest))) => {
            assert_eq!(v, None);
            drop(rest);
        },
        _ => unreachable!(),
    }
}

#[test]
fn it_recvs_after_close() {
    let mut tx = bus::Bus::new(1);
    let rx = tx.add_rx().into_future();
    assert_eq!(tx.try_broadcast(true), Ok(()));
    drop(tx);
    let rx = assert_poll!(rx, Some(true));
    drop(assert_poll!(rx, None));
}

#[test]
fn it_handles_leaves() {
    let mut c = bus::Bus::new(1);
    let r1 = c.add_rx().into_future();
    let r2 = c.add_rx().into_future();
    assert_eq!(c.try_broadcast(true), Ok(()));
    drop(r2);
    drop(assert_poll!(r1, Some(true)));
    assert_eq!(c.try_broadcast(true), Ok(()));
}

#[test]
fn it_runs_blocked_writes() {
    use std::thread;

    let mut c = Box::new(bus::Bus::new(1));
    let r1 = c.add_rx().into_future();
    c.broadcast(true); // this is fine
    // buffer is now full
    assert_eq!(c.try_broadcast(false), Err(false));
    // start other thread that blocks
    let c = thread::spawn(move || {
        c.broadcast(false);
    });
    // unblock sender by receiving
    let r1 = assert_poll!(r1, Some(true));
    // drop r1 to release other thread and safely drop c
    drop(r1);
    c.join().unwrap();
}

#[test]
fn it_runs_blocked_reads() {
    use std::thread;

    let mut tx = Box::new(bus::Bus::new(1));
    let rx = tx.add_rx().into_future();
    // buffer is now empty
    let mut rx = assert_nopoll!(rx);
    // start other thread that blocks
    let c = thread::spawn(move || {
        use std::sync::Arc;
        if let Ok(Async::Ready((Some(_), _))) = rx.poll_future(Arc::new(Sink)) {
        } else {
            unreachable!();
        }
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
    let r1 = c.add_rx();
    let j = thread::spawn(move || {
        for i in 0..10000 {
            println!("sent {}", i);
            c.broadcast(i);
        }
    });

    let mut r1 = r1.wait();
    for i in 0..10000 {
        let rx = r1.next().unwrap().unwrap();
        println!("recv {}", rx);
        assert_eq!(rx, i);
    }

    j.join().unwrap();
    assert_eq!(r1.next(), None);
}


#[test]
fn test_busy() {
    use std::thread;

    // start a bus with limited space
    let mut bus = bus::Bus::new(1);

    // first receiver only receives 5 items
    let rx1 = bus.add_rx();
    let t1 = thread::spawn(move || {
        let mut rx1 = rx1.wait();
        for _ in 0..5 {
            rx1.next().unwrap().unwrap();
        }
        drop(rx1);
    });

    // second receiver receives 10 items
    let rx2 = bus.add_rx();
    let t2 = thread::spawn(move || {
        let mut rx2 = rx2.wait();
        for _ in 0..10 {
            rx2.next().unwrap().unwrap();
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
