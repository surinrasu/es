#![no_std]
#![no_main]
#![feature(abi_avr_interrupt)]
#![feature(asm_experimental_arch)]

use panic_halt as _;

#[es_entry::module(e2)]
fn main() -> ! {
    entry::run();
}
