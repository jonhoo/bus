extern crate bus;
extern crate num_cpus;
extern crate futures;

use std::time::{Duration, Instant};
use bus::Bus;

const USE_TRY_RECV: bool = false;
const USE_TRY_SEND: bool = false;

fn helper(buf: usize, iter: usize, rxs: usize) -> u64 {
    use std::thread;

    let mut c = Bus::new(buf);
    let wait = (0..rxs)
        .map(|_| c.add_rx())
        .map(|mut rx| {
            thread::spawn(move || {
                use futures::stream::Stream;
                let w = Duration::new(0, 10);
                if USE_TRY_RECV {
                    loop {
                        match rx.poll() {
                            Ok(futures::Async::Ready(Some(true))) => break,
                            Ok(futures::Async::Ready(None)) => break,
                            Ok(futures::Async::NotReady) => thread::sleep(w),
                            _ => (),
                        }
                    }
                } else {
                    let mut rx = rx.wait();
                    loop {
                        match rx.next() {
                            Some(Ok(true)) => break,
                            None => break,
                            _ => (),
                        }
                    }
                }
            })
        })
        .collect::<Vec<_>>();

    let w = Duration::new(0, 10);
    let mut send = |v| {
        if USE_TRY_SEND {
            while let Err(..) = c.try_broadcast(v) {
                thread::sleep(w);
            }
        } else {
            c.broadcast(v);
        }
    };

    let start = Instant::now();
    for _ in 0..iter {
        send(false);
    }

    send(true);
    for w in wait.into_iter() {
        w.join().unwrap();
    }

    let dur = start.elapsed();
    (dur.as_secs() * 1000_000) + (dur.subsec_nanos() / 1000) as u64
}

fn main() {
    let num = 2_000_000;

    for threads in 1..(2 * num_cpus::get()) {
        println!("{} {} {:.*} μs/op",
                 threads,
                 1_000,
                 2,
                 helper(1_000, num, threads) as f64 / num as f64);
    }
}
