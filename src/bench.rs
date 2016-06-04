extern crate bus;
extern crate time;
extern crate num_cpus;

use bus::Bus;

const USE_TRY_RECV: bool = false;
const USE_TRY_SEND: bool = false;

fn helper(buf: usize, iter: usize, rxs: usize) -> u64 {
    use std::time::Duration;
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
                    } else {
                        if let Ok(true) = rx.recv() {
                            break;
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

    let start = time::precise_time_ns();
    for _ in 0..iter {
        send(false);
    }

    send(true);
    for w in wait.into_iter() {
        w.join().unwrap();
    }

    time::precise_time_ns() - start
}

fn main() {
    let num = 2_000_000;

    for threads in 1..(num_cpus::get() + 10) {
        println!("{} {} {:.*} μs/op",
                 threads,
                 1_000,
                 2,
                 helper(1_000, num, threads) as f64 / num as f64);
    }
}
