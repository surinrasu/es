use arduino_hal::{
    pac,
    port::{mode, Pin, PinOps},
};

pub(crate) const TRIGGER_PULSE_US: u16 = 10;
pub(crate) const DEFAULT_TIMEOUT_US: u32 = 200_000;
pub(crate) const TIMER1_TICK_US: u16 = 4;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) enum Sr04Error {
    NoPulseStart,
    NoPulseEnd,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub(crate) struct Sr04Reading {
    echo_ticks: u16,
}

impl Sr04Reading {
    pub(crate) const fn from_echo_ticks(echo_ticks: u16) -> Self {
        Self { echo_ticks }
    }

    pub(crate) const fn echo_us(self) -> u32 {
        self.echo_ticks as u32 * TIMER1_TICK_US as u32
    }

    pub(crate) const fn distance_mm(self) -> u16 {
        ((self.echo_us() * 10 + 29) / 58) as u16
    }

    pub(crate) const fn distance_cm(self) -> u16 {
        (self.distance_mm() + 5) / 10
    }
}

pub(crate) struct HsSr04<TRIG, ECHO, IMODE = mode::AnyInput>
where
    TRIG: PinOps,
    ECHO: PinOps,
    IMODE: mode::InputMode,
{
    trigger: Pin<mode::Output, TRIG>,
    echo: Pin<mode::Input<IMODE>, ECHO>,
    timer1: pac::TC1,
    timeout_ticks: u16,
}

impl<TRIG, ECHO, IMODE> HsSr04<TRIG, ECHO, IMODE>
where
    TRIG: PinOps,
    ECHO: PinOps,
    IMODE: mode::InputMode,
{
    pub(crate) fn new(
        mut trigger: Pin<mode::Output, TRIG>,
        echo: Pin<mode::Input<IMODE>, ECHO>,
        timer1: pac::TC1,
    ) -> Self {
        trigger.set_low();
        timer1.tccr1b().write(|w| w.cs1().prescale_64());

        Self {
            trigger,
            echo,
            timer1,
            timeout_ticks: micros_to_ticks(DEFAULT_TIMEOUT_US),
        }
    }

    pub(crate) fn set_timeout_us(&mut self, timeout_us: u32) {
        self.timeout_ticks = micros_to_ticks(timeout_us);
    }

    pub(crate) fn measure(&mut self) -> Result<Sr04Reading, Sr04Error> {
        self.trigger_pulse();
        self.wait_for_echo_start()?;
        let echo_ticks = self.wait_for_echo_end()?;
        Ok(Sr04Reading::from_echo_ticks(echo_ticks))
    }

    fn trigger_pulse(&mut self) {
        self.trigger.set_low();
        arduino_hal::delay_us(2);
        self.trigger.set_high();
        arduino_hal::delay_us(TRIGGER_PULSE_US as u32);
        self.trigger.set_low();
    }

    fn wait_for_echo_start(&mut self) -> Result<(), Sr04Error> {
        self.reset_timer();

        while self.echo.is_low() {
            if self.timer_ticks() >= self.timeout_ticks {
                return Err(Sr04Error::NoPulseStart);
            }
        }

        Ok(())
    }

    fn wait_for_echo_end(&mut self) -> Result<u16, Sr04Error> {
        self.reset_timer();

        while self.echo.is_high() {
            if self.timer_ticks() >= self.timeout_ticks {
                return Err(Sr04Error::NoPulseEnd);
            }
        }

        Ok(self.timer_ticks())
    }

    fn reset_timer(&mut self) {
        self.timer1.tcnt1().write(|w| w.set(0));
    }

    fn timer_ticks(&self) -> u16 {
        self.timer1.tcnt1().read().bits()
    }
}

const fn micros_to_ticks(micros: u32) -> u16 {
    let tick_us = TIMER1_TICK_US as u32;
    let rounded_up = micros.saturating_add(tick_us - 1) / tick_us;

    if rounded_up > u16::MAX as u32 {
        u16::MAX
    } else {
        rounded_up as u16
    }
}
