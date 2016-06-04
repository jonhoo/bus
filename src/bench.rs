extern crate bus;
extern crate time;
extern crate num_cpus;

use bus::Bus;

fn helper(buf: usize, iter: usize, rxs: usize) -> u64 {
    use std::thread;

    let mut c = Bus::new(buf);
    let wait = (0..rxs)
        .map(|_| c.add_rx())
        .map(|mut rx| {
            thread::spawn(move || {
                // use std::time::Duration;
                // let w = Duration::new(0, 10);
                loop {
                    if let Ok(true) = rx.recv() {
                        break;
                    }
                    // match rx.try_recv() {
                    // Ok(true) => break,
                    // Err(..) => thread::sleep(w),
                    // _ => (),
                    // }
                }
            })
        })
        .collect::<Vec<_>>();

    let start = time::precise_time_ns();
    for _ in 0..iter {
        c.broadcast(false)
    }

    c.broadcast(true);
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
