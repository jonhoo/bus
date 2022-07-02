# bus

[![Crates.io](https://img.shields.io/crates/v/bus.svg)](https://crates.io/crates/bus)
[![Documentation](https://docs.rs/bus/badge.svg)](https://docs.rs/bus/)
[![Codecov](https://codecov.io/github/jonhoo/bus/coverage.svg?branch=master)](https://codecov.io/gh/jonhoo/bus)

Bus provides a lock-free, bounded, single-producer, multi-consumer, broadcast channel.

**NOTE: bus sometimes busy-waits in the current implementation, which may cause increased CPU usage — see [#23](https://github.com/jonhoo/bus/issues/23).**

It uses a circular buffer and atomic instructions to implement a lock-free single-producer,
multi-consumer channel. The interface is similar to that of the `std::sync::mpsc` channels,
except that multiple consumers (readers of the channel) can be produced, whereas only a single
sender can exist. Furthermore, in contrast to most multi-consumer FIFO queues, bus is
*broadcast*; every send goes to every consumer.

I haven't seen this particular implementation in literature (some extra bookkeeping is
necessary to allow multiple consumers), but a lot of related reading can be found in Ross
Bencina's blog post ["Some notes on lock-free and wait-free
algorithms"](http://www.rossbencina.com/code/lockfree).

See [the documentation] for usage examples.

  [the documentation]: https://docs.rs/bus/

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
