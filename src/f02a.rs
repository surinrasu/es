use arduino_hal::{
    hal::port::PG5,
    pac,
    port::{mode, Pin},
};

const CPU_FREQ_HZ: u32 = 16_000_000;

type SignalPin = Pin<mode::Output, PG5>;

pub(crate) struct HsF02a {
    signal: SignalPin,
    timer2: pac::TC2,
    stop_at_ms: Option<u32>,
}

impl HsF02a {
    pub(crate) fn new(mut signal: SignalPin, timer2: pac::TC2) -> Self {
        signal.set_low();
        init_timer2(&timer2);

        Self {
            signal,
            timer2,
            stop_at_ms: None,
        }
    }

    pub(crate) fn play_tone(&mut self, frequency_hz: u16, duration_ms: u32, now_ms: u32) {
        if duration_ms == 0 || frequency_hz == 0 {
            self.silence();
            return;
        }

        let Some(config) = tone_config(frequency_hz) else {
            self.silence();
            return;
        };

        avr_device::interrupt::free(|_| {
            self.signal.set_low();
            stop_timer2(&self.timer2);

            self.timer2
                .ocr2a()
                .write(|w| unsafe { w.bits(config.compare) });
            self.timer2.tcnt2().write(|w| unsafe { w.bits(0) });
            self.timer2.tifr2().write(|w| w.ocf2a().set_bit());
            self.timer2.timsk2().modify(|_, w| w.ocie2a().set_bit());
            start_timer2(&self.timer2, config.prescaler);
        });

        self.stop_at_ms = Some(now_ms.wrapping_add(duration_ms));
    }

    pub(crate) fn poll(&mut self, now_ms: u32) {
        if let Some(stop_at_ms) = self.stop_at_ms {
            if time_reached(now_ms, stop_at_ms) {
                self.silence();
            }
        }
    }

    pub(crate) fn silence(&mut self) {
        avr_device::interrupt::free(|_| {
            stop_timer2(&self.timer2);
            self.signal.set_low();
        });

        self.stop_at_ms = None;
    }
}

#[avr_device::interrupt(atmega2560)]
fn TIMER2_COMPA() {
    let portg = unsafe { &*pac::PORTG::ptr() };
    portg.ping().write(|w| w.pg5().set_bit());
}

#[derive(Copy, Clone)]
struct ToneConfig {
    compare: u8,
    prescaler: Timer2Prescaler,
}

#[derive(Copy, Clone)]
enum Timer2Prescaler {
    Direct,
    Prescale8,
    Prescale32,
    Prescale64,
    Prescale128,
    Prescale256,
    Prescale1024,
}

impl Timer2Prescaler {
    const fn divisor(self) -> u32 {
        match self {
            Self::Direct => 1,
            Self::Prescale8 => 8,
            Self::Prescale32 => 32,
            Self::Prescale64 => 64,
            Self::Prescale128 => 128,
            Self::Prescale256 => 256,
            Self::Prescale1024 => 1024,
        }
    }
}

fn init_timer2(timer2: &pac::TC2) {
    timer2.gtccr().write(|w| w.psrasy().set_bit());
    timer2.timsk2().write(|w| {
        w.toie2().clear_bit();
        w.ocie2a().clear_bit();
        w.ocie2b().clear_bit()
    });
    timer2.tccr2a().write(|w| {
        w.wgm2().ctc();
        w.com2a().disconnected();
        w.com2b().disconnected()
    });
    timer2.tccr2b().write(|w| {
        w.wgm22().clear_bit();
        w.cs2().no_clock()
    });
    timer2.tcnt2().write(|w| unsafe { w.bits(0) });
    timer2.tifr2().write(|w| {
        w.tov2().set_bit();
        w.ocf2a().set_bit();
        w.ocf2b().set_bit()
    });
}

fn start_timer2(timer2: &pac::TC2, prescaler: Timer2Prescaler) {
    timer2.tccr2b().modify(|_, w| match prescaler {
        Timer2Prescaler::Direct => w.wgm22().clear_bit().cs2().direct(),
        Timer2Prescaler::Prescale8 => w.wgm22().clear_bit().cs2().prescale_8(),
        Timer2Prescaler::Prescale32 => w.wgm22().clear_bit().cs2().prescale_32(),
        Timer2Prescaler::Prescale64 => w.wgm22().clear_bit().cs2().prescale_64(),
        Timer2Prescaler::Prescale128 => w.wgm22().clear_bit().cs2().prescale_128(),
        Timer2Prescaler::Prescale256 => w.wgm22().clear_bit().cs2().prescale_256(),
        Timer2Prescaler::Prescale1024 => w.wgm22().clear_bit().cs2().prescale_1024(),
    });
}

fn stop_timer2(timer2: &pac::TC2) {
    timer2.timsk2().modify(|_, w| w.ocie2a().clear_bit());
    timer2.tccr2b().modify(|_, w| w.wgm22().clear_bit().cs2().no_clock());
    timer2.tcnt2().write(|w| unsafe { w.bits(0) });
    timer2.tifr2().write(|w| w.ocf2a().set_bit());
}

fn tone_config(frequency_hz: u16) -> Option<ToneConfig> {
    let frequency_hz = frequency_hz as u32;

    [
        Timer2Prescaler::Direct,
        Timer2Prescaler::Prescale8,
        Timer2Prescaler::Prescale32,
        Timer2Prescaler::Prescale64,
        Timer2Prescaler::Prescale128,
        Timer2Prescaler::Prescale256,
        Timer2Prescaler::Prescale1024,
    ]
    .into_iter()
    .find_map(|prescaler| {
        let divisor = prescaler.divisor() * frequency_hz * 2;
        let cycles = (CPU_FREQ_HZ + divisor / 2) / divisor;

        if cycles == 0 || cycles > u8::MAX as u32 + 1 {
            None
        } else {
            Some(ToneConfig {
                compare: (cycles - 1) as u8,
                prescaler,
            })
        }
    })
}

const fn time_reached(now_ms: u32, target_ms: u32) -> bool {
    now_ms.wrapping_sub(target_ms) < (1 << 31)
}
