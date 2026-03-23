# es
![](./banner.png)

This repo contains a serial of Rust programs for Arduino Mega 2560. Since this is merely a class assignment, it has been intentionally kept simple and straightforward, which may result in weaker robustness and maintainability.

> `es` stands for Embedded Systems. Although the actual name of this course is "开源硬件", all my friends living in English speaking countries told me they’ve never heard of a course called something like "Open Source Hardwares".

## Usage

You may use [mise](https://mise.jdx.dev/) to set up toolchain:

```shell
mise trust && mise install
```

Or if you prefer to install things manually:

```shell
# 1)
rustup toolchain install nightly-2025-04-27 --component rust-src
cargo install cargo-ravedude

# 2)
# Debian
sudo apt install avr-libc gcc-avr pkg-config avrdude libudev-dev build-essential

# macOS
xcode-select --install # if you haven't already done so
brew tap osx-cross/avr
brew install avr-gcc avrdude

# Windows
winget install AVRDudes.AVRDUDE ZakKemble.avr-gcc
```

Then you can just make a normal `cargo build` or directly `cargo run` if the board is already connected.

If automatic port detection does not work on your machine, you can use env varible `RAVEDUDE_PORT` to set explicitly. It usually looks like `/dev/cu.usbmodem2101` on Unix or `COM3` on Windows.

Currently the active entry is selected in `src/main.rs`:

```rust
#[es_entry::module(foo)]
fn main() -> ! {
    entry::run();
}
```

You may change the entry by switching the attribute argument:

```rust
#[es_entry::module(bar)]
```

## License

Published by [Rinsu Su](https://github.com/surinrasu) under [the MIT License](https://raw.githubusercontent.com/surinrasu/es/refs/heads/master/LICENSE).
