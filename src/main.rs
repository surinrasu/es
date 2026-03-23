#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]
#![feature(asm_experimental_arch)]

use panic_halt as _;

#[es_entry::module(e1)]
fn main() -> ! {
    entry::run();
}
