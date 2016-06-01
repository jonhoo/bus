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
                loop {
                    if let Ok(true) = unsafe { rx.recv() } {
                        break;
                    }
                    thread::yield_now();
                }
            })
        })
        .collect::<Vec<_>>();

    let start = time::precise_time_ns();
    for _ in 0..iter {
        while let Err(_) = c.broadcast(false) {
            thread::yield_now();
        }
    }

    while let Err(_) = c.broadcast(true) {
        thread::yield_now();
    }

    for w in wait.into_iter() {
        w.join().unwrap();
    }

    time::precise_time_ns() - start
}

fn main() {
    let num = 2_000_000;

    println!("1 {} {:.*} μs/op",
             10,
             2,
             helper(10, num, 1) as f64 / num as f64);

    for threads in 1..num_cpus::get() {
        println!("{} {} {:.*} μs/op",
                 threads,
                 1_000,
                 2,
                 helper(1_000, num, threads) as f64 / num as f64);
    }
}
