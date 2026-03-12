#![no_std]
#![no_main]
#![feature(asm_experimental_arch)]

use panic_halt as _;
use crate::w2 as entry;

#[arduino_hal::entry]
fn main() -> ! {
    entry::run();
}

mod f03a;
mod w2;
