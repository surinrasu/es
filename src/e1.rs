use core::cell::Cell;

use smart_leds::RGB8;

#[es_sim::export]
const LEARN_STEPS: usize = 4;
#[es_sim::export]
const BRIGHTNESS_LEVELS: [u8; 3] = [96, 160, 255];
const PALETTE_LIGHTNESS: f32 = 68.0;
const PALETTE_CHROMA: f32 = 52.0;
const PALETTE_STEP_DEGREES: f32 = 82.0;
const LEARN_MIN_GAP_MS: u32 = 90;
const LEARN_MAX_GAP_MS: u32 = 900;
const LEARN_BLINK_MS: u32 = 180;
const RNG_FALLBACK_SEED: u32 = 0xC0DE_2560;
#[es_sim::export]
const DEFAULT_GAPS_MS: [u16; LEARN_STEPS] = [220, 150, 260, 340];
const TIMER0_PRESCALER: u32 = 64;
const TIMER0_COUNTS: u8 = 250;
const MILLIS_INCREMENT: u32 = 1;
const INT2_ENABLE_BIT: u8 = register_bit(2);
const INT3_ENABLE_BIT: u8 = register_bit(3);
const INT4_ENABLE_BIT: u8 = register_bit(4);
const INT5_ENABLE_BIT: u8 = register_bit(5);
const BUTTON_INT_ENABLE_MASK: u8 =
    INT2_ENABLE_BIT | INT3_ENABLE_BIT | INT4_ENABLE_BIT | INT5_ENABLE_BIT;

static MILLIS_COUNTER: avr_device::interrupt::Mutex<Cell<u32>> =
    avr_device::interrupt::Mutex::new(Cell::new(0));

#[derive(Copy, Clone, Eq, PartialEq)]
enum Effect {
    Orbit,
    Tide,
}

impl Effect {
    fn next(self) -> Self {
        match self {
            Self::Orbit => Self::Tide,
            Self::Tide => Self::Orbit,
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum Direction {
    Forward,
    Reverse,
}

impl Direction {
    fn toggled(self) -> Self {
        match self {
            Self::Forward => Self::Reverse,
            Self::Reverse => Self::Forward,
        }
    }
}

#[derive(Copy, Clone)]
struct RhythmPattern {
    gaps_ms: [u16; LEARN_STEPS],
    accents: [u8; LEARN_STEPS],
}

impl RhythmPattern {
    const fn default() -> Self {
        Self {
            gaps_ms: DEFAULT_GAPS_MS,
            accents: [
                crate::key4b::SW1 as u8,
                crate::key4b::SW2 as u8,
                crate::key4b::SW3 as u8,
                crate::key4b::SW4 as u8,
            ],
        }
    }
}

#[derive(Copy, Clone)]
struct LearnCapture {
    count: usize,
    last_tap_ms: u32,
    gaps_ms: [u16; LEARN_STEPS],
    accents: [u8; LEARN_STEPS],
}

impl LearnCapture {
    fn new(now_ms: u32) -> Self {
        Self {
            count: 0,
            last_tap_ms: now_ms,
            gaps_ms: DEFAULT_GAPS_MS,
            accents: RhythmPattern::default().accents,
        }
    }
}

enum SceneMode {
    Perform,
    Learn(LearnCapture),
}

struct XorShift32 {
    state: u32,
}

impl XorShift32 {
    const fn new(seed: u32) -> Self {
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

    fn next_unit_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}

struct LanternController {
    effect: Effect,
    direction: Direction,
    rhythm: RhythmPattern,
    key_colors: [RGB8; crate::key4b::KEY_N],
    brightness_index: usize,
    mode: SceneMode,
    beat_index: usize,
    animation_step: usize,
    next_step_ms: u32,
    learn_blink_phase: u32,
}

impl LanternController {
    fn new(rng: &mut XorShift32, now_ms: u32) -> Self {
        let mut key_colors = [crate::f12a::OFF; crate::key4b::KEY_N];
        crate::f12a::fill_spaced_palette(
            &mut key_colors,
            rng.next_unit_f32() * 360.0,
            PALETTE_LIGHTNESS,
            PALETTE_CHROMA,
            PALETTE_STEP_DEGREES,
        );

        let rhythm = RhythmPattern::default();
        Self {
            effect: Effect::Orbit,
            direction: Direction::Forward,
            rhythm,
            key_colors,
            brightness_index: 1,
            mode: SceneMode::Perform,
            beat_index: 0,
            animation_step: 0,
            next_step_ms: now_ms + rhythm.gaps_ms[0] as u32,
            learn_blink_phase: now_ms / LEARN_BLINK_MS,
        }
    }

    fn handle_short_press(&mut self, key: usize, now_ms: u32, rng: &mut XorShift32) {
        match self.mode {
            SceneMode::Perform => match key {
                crate::key4b::SW1 => self.effect = self.effect.next(),
                crate::key4b::SW2 => self.randomize_palette(rng),
                crate::key4b::SW3 => self.direction = self.direction.toggled(),
                crate::key4b::SW4 => {
                    self.brightness_index = (self.brightness_index + 1) % BRIGHTNESS_LEVELS.len();
                }
                _ => {}
            },
            SceneMode::Learn(mut capture) => {
                let gap_ms = clamp_gap(now_ms - capture.last_tap_ms);
                capture.gaps_ms[capture.count] = gap_ms as u16;
                capture.accents[capture.count] = key as u8;
                capture.last_tap_ms = now_ms;
                capture.count += 1;

                if capture.count >= LEARN_STEPS {
                    self.rhythm = RhythmPattern {
                        gaps_ms: capture.gaps_ms,
                        accents: capture.accents,
                    };
                    self.mode = SceneMode::Perform;
                    self.reset_transport(now_ms);
                } else {
                    self.mode = SceneMode::Learn(capture);
                }
            }
        }
    }

    fn handle_long_press(&mut self, key: usize, now_ms: u32) {
        match self.mode {
            SceneMode::Perform => match key {
                crate::key4b::SW1 => {
                    self.mode = SceneMode::Learn(LearnCapture::new(now_ms));
                    self.learn_blink_phase = now_ms / LEARN_BLINK_MS;
                }
                crate::key4b::SW4 => {
                    self.rhythm = RhythmPattern::default();
                    self.mode = SceneMode::Perform;
                    self.reset_transport(now_ms);
                }
                _ => {}
            },
            SceneMode::Learn(_) => {
                if key == crate::key4b::SW1 {
                    self.mode = SceneMode::Perform;
                    self.reset_transport(now_ms);
                }
            }
        }
    }

    fn tick(&mut self, now_ms: u32) -> bool {
        match self.mode {
            SceneMode::Perform => {
                if now_ms < self.next_step_ms {
                    return false;
                }

                self.animation_step = self.animation_step.wrapping_add(1);
                self.beat_index = (self.beat_index + 1) % LEARN_STEPS;
                self.next_step_ms = now_ms + self.rhythm.gaps_ms[self.beat_index] as u32;
                true
            }
            SceneMode::Learn(_) => {
                let phase = now_ms / LEARN_BLINK_MS;

                if phase == self.learn_blink_phase {
                    return false;
                }

                self.learn_blink_phase = phase;
                true
            }
        }
    }

    fn render(&self, frame: &mut [RGB8; crate::f12a::LED_N], now_ms: u32) {
        match &self.mode {
            SceneMode::Perform => match self.effect {
                Effect::Orbit => self.render_orbit(frame),
                Effect::Tide => self.render_tide(frame),
            },
            SceneMode::Learn(capture) => self.render_learning(frame, capture, now_ms),
        }
    }

    fn render_orbit(&self, frame: &mut [RGB8; crate::f12a::LED_N]) {
        frame.fill(crate::f12a::OFF);

        for slot in 0..LEARN_STEPS {
            let base = (self.animation_step + slot * 3) % crate::f12a::LED_N;
            let head = map_index(base, self.direction);
            let tail = map_index(
                (base + crate::f12a::LED_N - 1) % crate::f12a::LED_N,
                self.direction,
            );
            let accent = self.rhythm.accents[(self.beat_index + slot) % LEARN_STEPS] as usize;
            let color = self.bright_color(accent);
            frame[head] = color;
            frame[tail] = scale_rgb(color, 96);
        }
    }

    fn render_tide(&self, frame: &mut [RGB8; crate::f12a::LED_N]) {
        frame.fill(crate::f12a::OFF);

        let cycle = crate::f12a::LED_N * 2;
        let phase = self.animation_step % cycle;
        let fill_n = if phase < crate::f12a::LED_N {
            phase + 1
        } else {
            cycle - phase - 1
        };

        for index in 0..fill_n {
            let pos = map_index(index, self.direction);
            let accent = self.rhythm.accents[(self.beat_index + index) % LEARN_STEPS] as usize;
            frame[pos] = self.bright_color(accent);
        }

        if fill_n < crate::f12a::LED_N {
            let guide = map_index(fill_n, self.direction);
            let accent = self.rhythm.accents[self.beat_index] as usize;
            frame[guide] = scale_rgb(self.bright_color(accent), 72);
        }
    }

    fn render_learning(
        &self,
        frame: &mut [RGB8; crate::f12a::LED_N],
        capture: &LearnCapture,
        now_ms: u32,
    ) {
        frame.fill(RGB8 { r: 0, g: 0, b: 0 });

        for slot in 0..LEARN_STEPS {
            let start = slot * 3;
            let center = start + 1;

            if slot < capture.count {
                let color = self.bright_color(capture.accents[slot] as usize);
                frame[start] = scale_rgb(color, 96);
                frame[center] = color;
                frame[start + 2] = scale_rgb(color, 96);
            } else if slot == capture.count && (now_ms / LEARN_BLINK_MS) % 2 == 0 {
                frame[center] = scale_rgb(
                    RGB8 {
                        r: 64,
                        g: 64,
                        b: 64,
                    },
                    BRIGHTNESS_LEVELS[self.brightness_index],
                );
            } else {
                frame[center] = RGB8 { r: 0, g: 0, b: 4 };
            }
        }
    }

    fn reset_transport(&mut self, now_ms: u32) {
        self.beat_index = 0;
        self.animation_step = 0;
        self.next_step_ms = now_ms + self.rhythm.gaps_ms[0] as u32;
        self.learn_blink_phase = now_ms / LEARN_BLINK_MS;
    }

    fn randomize_palette(&mut self, rng: &mut XorShift32) {
        crate::f12a::fill_spaced_palette(
            &mut self.key_colors,
            rng.next_unit_f32() * 360.0,
            PALETTE_LIGHTNESS,
            PALETTE_CHROMA,
            PALETTE_STEP_DEGREES,
        );
    }

    fn bright_color(&self, accent: usize) -> RGB8 {
        scale_rgb(
            self.key_colors[accent % crate::key4b::KEY_N],
            BRIGHTNESS_LEVELS[self.brightness_index],
        )
    }
}

#[inline(always)]
const fn register_bit(bit: u8) -> u8 {
    1u8 << bit
}

fn map_index(index: usize, direction: Direction) -> usize {
    match direction {
        Direction::Forward => index,
        Direction::Reverse => crate::f12a::LED_N - 1 - index,
    }
}

fn clamp_gap(value_ms: u32) -> u32 {
    value_ms.clamp(LEARN_MIN_GAP_MS, LEARN_MAX_GAP_MS)
}

fn scale_rgb(color: RGB8, level: u8) -> RGB8 {
    RGB8 {
        r: ((color.r as u16 * level as u16) / 255) as u8,
        g: ((color.g as u16 * level as u16) / 255) as u8,
        b: ((color.b as u16 * level as u16) / 255) as u8,
    }
}

fn millis_init(tc0: arduino_hal::pac::TC0) {
    tc0.tccr0a().write(|w| w.wgm0().ctc());
    tc0.ocr0a().write(|w| w.set(TIMER0_COUNTS));
    tc0.tccr0b().write(|w| match TIMER0_PRESCALER {
        8 => w.cs0().prescale_8(),
        64 => w.cs0().prescale_64(),
        256 => w.cs0().prescale_256(),
        1024 => w.cs0().prescale_1024(),
        _ => panic!(),
    });
    tc0.timsk0().write(|w| w.ocie0a().set_bit());

    avr_device::interrupt::free(|cs| {
        MILLIS_COUNTER.borrow(cs).set(0);
    });
}

fn millis() -> u32 {
    avr_device::interrupt::free(|cs| MILLIS_COUNTER.borrow(cs).get())
}

fn seed_rng() -> u32 {
    let timer_seed = unsafe { (*arduino_hal::pac::TC0::ptr()).tcnt0().read().bits() as u32 };
    timer_seed ^ RNG_FALLBACK_SEED
}

fn configure_button_interrupts(exint: &arduino_hal::pac::EXINT) {
    exint
        .eicra()
        .write(|w| w.isc2().val_0x01().isc3().val_0x01());
    exint
        .eicrb()
        .write(|w| w.isc4().val_0x01().isc5().val_0x01());
    exint
        .eimsk()
        .write(|w| w.int().set(BUTTON_INT_ENABLE_MASK));
}

fn configure_sleep(cpu: &arduino_hal::pac::CPU) {
    cpu.smcr().write(|w| w.sm().idle().se().set_bit());
}

fn timer_tick() {
    let now_ms = avr_device::interrupt::free(|cs| {
        let millis_cell = MILLIS_COUNTER.borrow(cs);
        let now_ms = millis_cell.get().wrapping_add(MILLIS_INCREMENT);
        millis_cell.set(now_ms);
        now_ms
    });

    crate::key4b::tick(now_ms);
}

#[inline(always)]
fn handle_button_interrupt(key: usize) {
    crate::key4b::schedule_debounce(key, millis());
}

#[avr_device::interrupt(atmega2560)]
fn INT2() {
    handle_button_interrupt(crate::key4b::SW4);
}

#[avr_device::interrupt(atmega2560)]
fn INT3() {
    handle_button_interrupt(crate::key4b::SW3);
}

#[avr_device::interrupt(atmega2560)]
fn INT4() {
    handle_button_interrupt(crate::key4b::SW1);
}

#[avr_device::interrupt(atmega2560)]
fn INT5() {
    handle_button_interrupt(crate::key4b::SW2);
}

#[avr_device::interrupt(atmega2560)]
fn TIMER0_COMPA() {
    timer_tick();
}

pub(crate) fn run() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    let mut panel = crate::f12a::HsF12a::new(pins.d51.into_output());

    let _ = pins.d2.into_pull_up_input();
    let _ = pins.d3.into_pull_up_input();
    let _ = pins.d18.into_pull_up_input();
    let _ = pins.d19.into_pull_up_input();

    millis_init(dp.TC0);
    crate::key4b::init_runtime();
    configure_button_interrupts(&dp.EXINT);
    configure_sleep(&dp.CPU);

    let mut rng = XorShift32::new(seed_rng());
    let mut controller = LanternController::new(&mut rng, millis());
    let mut frame = crate::f12a::blank_frame();

    controller.render(&mut frame, millis());
    panel.write(&frame);

    unsafe {
        avr_device::interrupt::enable();
    }

    loop {
        avr_device::asm::sleep();

        let now_ms = millis();
        let events = crate::key4b::drain_events();
        let mut dirty = false;

        for (index, event) in events.iter().enumerate() {
            match event {
                crate::key4b::ButtonEvent::Short => {
                    controller.handle_short_press(index, now_ms, &mut rng);
                    dirty = true;
                }
                crate::key4b::ButtonEvent::Long => {
                    controller.handle_long_press(index, now_ms);
                    dirty = true;
                }
                crate::key4b::ButtonEvent::None => {}
            }
        }

        if controller.tick(now_ms) {
            dirty = true;
        }

        if dirty {
            controller.render(&mut frame, now_ms);
            panel.write(&frame);
        }
    }
}

#[es_sim::test]
mod sim {
    use es_sim::prelude::*;

    use crate::f12a::LED_DATA_PIN;
    use crate::key4b::{DEBOUNCE_MS, SW1_PIN, SW2_PIN, SW3_PIN, SW4_PIN};

    fn release_all_buttons(sim: &mut Sim) {
        sim.set_pin(SW1_PIN, true);
        sim.set_pin(SW2_PIN, true);
        sim.set_pin(SW3_PIN, true);
        sim.set_pin(SW4_PIN, true);
    }

    #[es_sim::test(timeout_ms = 50)]
    fn power_on_render_drives_led_data(sim: &mut Sim) {
        release_all_buttons(sim);

        let transitions = sim.capture_pin_transitions(LED_DATA_PIN, 25_000, 128);

        assert!(!transitions.is_empty());
        assert!(transitions.iter().any(|transition| transition.level));
    }

    #[es_sim::test(timeout_ms = 120)]
    fn short_press_sw1_triggers_led_refresh(sim: &mut Sim) {
        release_all_buttons(sim);

        let startup_transitions = sim.capture_pin_transitions(LED_DATA_PIN, 25_000, 1_024);
        assert!(!startup_transitions.is_empty());

        sim.advance_ms(10);
        assert_eq!(sim.count_pin_edges(LED_DATA_PIN, 1_000), 0);

        sim.set_pin(SW1_PIN, false);
        sim.advance_ms(DEBOUNCE_MS as u64 + 5);
        sim.set_pin(SW1_PIN, true);

        let press_transitions =
            sim.capture_pin_transitions(LED_DATA_PIN, (DEBOUNCE_MS as u64 + 10) * 1_000, 128);

        assert!(!press_transitions.is_empty());
        assert!(sim.elapsed_ms() < super::DEFAULT_GAPS_MS[0] as u64);
    }
}
