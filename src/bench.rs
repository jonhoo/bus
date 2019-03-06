extern crate bus;
extern crate num_cpus;

use bus::Bus;
use std::time::{Duration, Instant};

const USE_TRY_RECV: bool = false;
const USE_TRY_SEND: bool = false;

fn helper(buf: usize, iter: usize, rxs: usize) -> u64 {
    use std::thread;

    let mut c = Bus::new(buf);
    let wait = (0..rxs)
        .map(|_| c.add_rx())
        .map(|mut rx| {
            thread::spawn(move || {
                let w = Duration::new(0, 10);
                loop {
                    if USE_TRY_RECV {
                        match rx.try_recv() {
                            Ok(true) => break,
                            Err(..) => thread::sleep(w),
                            _ => (),
                        }
                    } else if let Ok(true) = rx.recv() {
                        break;
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
    for w in wait {
        w.join().unwrap();
    }

    let dur = start.elapsed();
    (dur.as_secs() * 1_000_000) + u64::from(dur.subsec_nanos() / 1000)
}

fn main() {
    let num = 2_000_000;

    for threads in 1..(2 * num_cpus::get()) {
        println!(
            "{} {} {:.*} μs/op",
            threads,
            1_000,
            2,
            helper(1_000, num, threads) as f64 / num as f64
        );
    }
}
