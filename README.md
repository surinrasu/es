# es
![](./banner.png)

This repo contains a serial of Rust programs for Arduino Mega 2560. Since this is merely a class assignment, it has been intentionally kept simple and straightforward, which may result in weaker robustness and maintainability.

> `es` stands for embedded systems. Although the actual name of this course is "开源硬件", all my friends living in English speaking countries told me they’ve never heard of a course called something like "Open Source Hardware".

## Usage

You may use [mise](https://mise.jdx.dev/) to set up environment:

```shell
mise trust && mise install
```

Or if you prefer to install things manually:

```shell
rustup toolchain install nightly-2025-04-27 --component rust-src
cargo install cargo-ravedude

# Debain
sudo apt install avr-libc gcc-avr pkg-config avrdude libudev-dev build-essential

# macOS
xcode-select --install # if you haven't already done so
brew tap osx-cross/avr
brew install avr-gcc avrdude

# Windows
winget install AVRDudes.AVRDUDE ZakKemble.avr-gcc
```

Then you can just make a normal `cargo build` or directly `cargo run` if the board is already connected.

If automatic port detection does not work on your machine, you can point `ravedude` to the port explicitly:

```shell
RAVEDUDE_PORT=/dev/cu.usbmodem2101 cargo run --release
```

On Windows, the port name will usually look like `COM3` or `COM4`.

Right now the active entry is selected in `src/main.rs`:

```rust
use crate::w2 as entry;
```

You can change the entry module with a different use:

```rust
use crate::test as entry
```

## License

This repo is licensed under the MIT License.
