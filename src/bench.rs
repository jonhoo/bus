extern crate bus;
extern crate time;

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
    let num = 10_000_000;

    println!("1 {} {:.*}ns/op",
             10,
             3,
             helper(10, num, 1) as f64 / num as f64);

    println!("1 {} {:.*}ns/op",
             10_000,
             3,
             helper(10_000, num, 1) as f64 / num as f64);

    println!("2 {} {:.*}ns/op",
             10_000,
             3,
             helper(10_000, num, 2) as f64 / num as f64);

    println!("3 {} {:.*}ns/op",
             10_000,
             3,
             helper(10_000, num, 3) as f64 / num as f64);

    println!("4 {} {:.*}ns/op",
             10_000,
             3,
             helper(10_000, num, 4) as f64 / num as f64);
}
