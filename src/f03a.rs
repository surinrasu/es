use arduino_hal::port::{mode, Pin};

pub(crate) const LED_N: usize = 12;
pub(crate) const OFF: bool = false;
pub(crate) const ON: bool = true;

const RNG_FALLBACK_SEED: u32 = 0xC0DE_030A;

pub(crate) const fn blank_frame() -> [bool; LED_N] {
    [OFF; LED_N]
}

pub(crate) struct HsF03a {
    leds: [Pin<mode::Output>; LED_N],
}

impl HsF03a {
    pub(crate) fn new(mut leds: [Pin<mode::Output>; LED_N]) -> Self {
        for led in leds.iter_mut() {
            led.set_low();
        }

        Self { leds }
    }

    pub(crate) fn write(&mut self, frame: &[bool; LED_N]) {
        for (led, &on) in self.leds.iter_mut().zip(frame.iter()) {
            if on {
                led.set_high();
            } else {
                led.set_low();
            }
        }
    }
}

pub(crate) struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    pub(crate) const fn new(seed: u32) -> Self {
        let state = if seed == 0 { RNG_FALLBACK_SEED } else { seed };
        Self { state }
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;

        if x == 0 {
            x = RNG_FALLBACK_SEED;
        }

        self.state = x;
        x
    }

    pub(crate) fn next_index(&mut self, upper: usize) -> usize {
        (self.next_u32() as usize) % upper
    }
}

pub(crate) fn seed_rng() -> u32 {
    let timer_seed = unsafe { (*arduino_hal::pac::TC0::ptr()).tcnt0().read().bits() as u32 };
    timer_seed ^ RNG_FALLBACK_SEED
}

pub(crate) fn pick_distinct_leds<const N: usize>(
    rng: &mut XorShift32,
    active_leds: &mut [usize; N],
) {
    let mut candidate_leds: [usize; LED_N] = core::array::from_fn(|index| index);
    let count = core::cmp::min(N, LED_N);

    for slot in 0..count {
        let pick = slot + rng.next_index(LED_N - slot);
        candidate_leds.swap(slot, pick);
        active_leds[slot] = candidate_leds[slot];
    }
}

pub(crate) fn render_phase(
    frame: &mut [bool; LED_N],
    active_leds: &[usize],
    phase: usize,
) {
    frame.fill(OFF);

    for (slot, &led) in active_leds.iter().enumerate() {
        if slot % 2 == phase % 2 {
            frame[led] = ON;
        }
    }
}
