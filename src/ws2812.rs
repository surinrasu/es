use core::{arch::asm, convert::Infallible};

use arduino_hal::port::{mode, Pin, PinOps};
use smart_leds::{RGB8, SmartLedsWrite};

const MEGA2560_D51_PORT_IO: u8 = 0x05;
const MEGA2560_D51_PIN_BIT: u8 = 2;
const LATCH_DELAY_US: u32 = 80;

pub(crate) struct Mega2560Ws2812<PIN> {
    data: Pin<mode::Output, PIN>,
}

impl<PIN: PinOps> Mega2560Ws2812<PIN> {
    pub(crate) fn new(mut data: Pin<mode::Output, PIN>) -> Self {
        data.set_low();
        Self { data }
    }

    #[inline(always)]
    unsafe fn write_byte(byte: u8) {
        asm!(
            "ldi {count}, 8",
            "2:",
            "sbi {port}, {bit}",
            "nop",
            "sbrs {byte}, 7",
            "cbi {port}, {bit}",
            "nop",
            "nop",
            "sbrc {byte}, 7",
            "cbi {port}, {bit}",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "nop",
            "lsl {byte}",
            "dec {count}",
            "brne 2b",
            byte = inout(reg_upper) byte => _,
            count = lateout(reg_upper) _,
            port = const MEGA2560_D51_PORT_IO,
            bit = const MEGA2560_D51_PIN_BIT,
            options(nostack),
        );
    }

    fn latch(&mut self) {
        self.data.set_low();
        arduino_hal::delay_us(LATCH_DELAY_US);
    }
}

impl<PIN: PinOps> SmartLedsWrite for Mega2560Ws2812<PIN> {
    type Error = Infallible;
    type Color = RGB8;

    fn write<T, I>(&mut self, iterator: T) -> Result<(), Self::Error>
    where
        T: IntoIterator<Item = I>,
        I: Into<Self::Color>,
    {
        avr_device::interrupt::free(|_| unsafe {
            for pixel in iterator {
                let pixel = pixel.into();
                Self::write_byte(pixel.g);
                Self::write_byte(pixel.r);
                Self::write_byte(pixel.b);
            }
        });

        self.latch();
        Ok(())
    }
}
