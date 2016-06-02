extern crate bus;

#[test]
fn it_works() {
    let mut c = bus::Bus::new(10);
    let mut r1 = c.add_rx();
    let mut r2 = c.add_rx();
    assert_eq!(c.try_broadcast(true), Ok(()));
    assert_eq!(r1.recv(), Ok(true));
    assert_eq!(r2.recv(), Ok(true));
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
    assert_eq!(r1.recv(), Ok(true));
    assert_eq!(c.try_broadcast(true), Ok(()));
}

#[test]
fn it_fails_when_empty() {
    let mut c = bus::Bus::<bool>::new(10);
    let mut r1 = c.add_rx();
    assert_eq!(r1.recv(), Err(()));
}

#[test]
fn it_reads_when_full() {
    let mut c = bus::Bus::new(1);
    let mut r1 = c.add_rx();
    assert_eq!(c.try_broadcast(true), Ok(()));
    assert_eq!(r1.recv(), Ok(true));
}

#[test]
fn it_handles_leaves() {
    let mut c = bus::Bus::new(1);
    let mut r1 = c.add_rx();
    let r2 = c.add_rx();
    assert_eq!(c.try_broadcast(true), Ok(()));
    drop(r2);
    assert_eq!(r1.recv(), Ok(true));
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
        c.broadcast(false); // can't let c be dropped before r1 is
    });
    // unblock sender by receiving
    assert_eq!(r1.recv(), Ok(true));
    // drop r1 to release other thread and safely drop c
    drop(r1);
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
        loop {
            match r1.recv() {
                Ok(v) => {
                    assert_eq!(v, i);
                    break;
                }
                Err(..) => thread::yield_now(),
            }
        }
    }

    j.join().unwrap();
    assert_eq!(r1.recv(), Err(()));
}
