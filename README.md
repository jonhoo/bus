# bus

[![Build Status](https://travis-ci.org/jonhoo/bus.svg?branch=master)](https://travis-ci.org/jonhoo/bus)

[Documentation](https://jon.tsp.io/crates/bus)

Bus provides a lock-free, bounded, single-producer, multi-consumer, broadcast channel.

It uses a circular buffer and atomic instructions to implement a lock-free single-producer,
multi-consumer channel. The interface is similar to that of the `std::sync::mpsc` channels,
except that multiple consumers (readers of the channel) can be produced, whereas only a single
sender can exist. Furthermore, in contrast to most multi-consumer FIFO queues, bus is
*broadcast*; every send goes to every consumer.

I haven't seen this particular implementation in literature (some extra bookkeeping is
necessary to allow multiple consumers), but a lot of related reading can be found in Ross
Bencina's blog post ["Some notes on lock-free and wait-free
algorithms"](http://www.rossbencina.com/code/lockfree).

Bus achieves broadcast by cloning the element in question, which is why `T` must implement
`Clone`. However, Bus is clever about only cloning when necessary. Specifically, the last
consumer to see a given value will move it instead of cloning, which means no cloning is
happening for the single-consumer case. For cases where cloning is expensive, `Arc` should be
used instead.

In a single-producer, single-consumer setup (which is the only one that Bus and
`mpsc::sync_channel` both support), Bus gets ~2x the performance of `mpsc::sync_channel` on my
machine. YMMV. You can check your performance using

```console
$ cargo bench
```

To see multi-consumer results, run the benchmark utility instead

```console
$ cargo build --bin bench --release
$ target/release/bench
```

## Examples

Single-send, multi-consumer example

```rust
use bus::Bus;
let mut bus = Bus::new(10);
let mut rx1 = bus.add_rx();
let mut rx2 = bus.add_rx();

bus.broadcast("Hello");
assert_eq!(rx1.try_recv(), Ok("Hello"));
assert_eq!(rx2.try_recv(), Ok("Hello"));
```

Multi-send, multi-consumer example

```rust
use bus::Bus;
use std::thread;

let mut bus = Bus::new(10);
let mut rx1 = bus.add_rx();
let mut rx2 = bus.add_rx();

// start a thread that sends 1..100
let j = thread::spawn(move || {
    for i in 1..100 {
        bus.broadcast(i);
    }
});

// every value should be received by both receivers
for i in 1..100 {
    // rx1
    loop {
        // we don't yet have blocking receive, so we receive in a loop instead
        match rx1.try_recv() {
            Ok(msg) => {
                assert_eq!(msg, i);
                break;
            }
            Err(..) => thread::yield_now(),
        }
    }
    // and rx2
    loop {
        match rx2.try_recv() {
            Ok(msg) => {
                assert_eq!(msg, i);
                break;
            }
            Err(..) => thread::yield_now(),
        }
    }
}
```

Many-to-many channel using a dispatcher

```rust
use bus::Bus;

use std::thread;
use std::sync::mpsc;

// set up fan-in
let (tx1, mix_rx) = mpsc::sync_channel(100);
let tx2 = tx1.clone();
// set up fan-out
let mut mix_tx = Bus::new(100);
let mut rx1 = mix_tx.add_rx();
let mut rx2 = mix_tx.add_rx();
// start dispatcher
thread::spawn(move || {
    for m in mix_rx.iter() {
        mix_tx.broadcast(m);
    }
});

// sends on tx1 are received ...
tx1.send("Hello").unwrap();

// ... by both receiver rx1 ...
loop {
    // we don't yet have blocking receive, so we receive in a loop instead
    match rx1.try_recv() {
        Ok(msg) => {
            assert_eq!(msg, "Hello");
            break;
        }
        Err(..) => thread::yield_now(),
    }
}
// ... and receiver rx2
assert_eq!(rx2.try_recv(), Ok("Hello"));

// same with sends on tx2
tx2.send("world").unwrap();
loop {
    match rx1.try_recv() {
        Ok(msg) => {
            assert_eq!(msg, "world");
            break;
        }
        Err(..) => thread::yield_now(),
    }
}
assert_eq!(rx2.try_recv(), Ok("world"));
```
