#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

use panic_halt as _;

#[es_entry::module(w2)]
fn main() -> ! {
    entry::run();
}
