extern crate bus;

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
        for i in 0..10000 {
            c.broadcast(i);
        }
    });

    for i in 0..10000 {
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
