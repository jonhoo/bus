extern crate bus;

#[test]
fn it_works() {
    let mut c = bus::Bus::new(10);
    let mut r1 = c.add_rx();
    let mut r2 = c.add_rx();
    assert_eq!(c.broadcast(true), Ok(()));
    assert_eq!(unsafe { r1.recv() }, Ok(true));
    assert_eq!(unsafe { r2.recv() }, Ok(true));
}

#[test]
fn it_fails_when_full() {
    let mut c = bus::Bus::new(1);
    let r1 = c.add_rx();
    assert_eq!(c.broadcast(true), Ok(()));
    assert_eq!(c.broadcast(false), Err(false));
    drop(r1);
}

#[test]
fn it_succeeds_when_not_full() {
    let mut c = bus::Bus::new(1);
    let mut r1 = c.add_rx();
    assert_eq!(c.broadcast(true), Ok(()));
    assert_eq!(c.broadcast(false), Err(false));
    assert_eq!(unsafe { r1.recv() }, Ok(true));
    assert_eq!(c.broadcast(true), Ok(()));
}

#[test]
fn it_fails_when_empty() {
    let mut c = bus::Bus::<bool>::new(10);
    let mut r1 = c.add_rx();
    assert_eq!(unsafe { r1.recv() }, Err(()));
}

#[test]
fn it_reads_when_full() {
    let mut c = bus::Bus::new(1);
    let mut r1 = c.add_rx();
    assert_eq!(c.broadcast(true), Ok(()));
    assert_eq!(unsafe { r1.recv() }, Ok(true));
}

#[test]
fn it_handles_leaves() {
    let mut c = bus::Bus::new(1);
    let mut r1 = c.add_rx();
    let r2 = c.add_rx();
    assert_eq!(c.broadcast(true), Ok(()));
    drop(r2);
    assert_eq!(unsafe { r1.recv() }, Ok(true));
    assert_eq!(c.broadcast(true), Ok(()));
}
