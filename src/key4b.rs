use core::cell::Cell;

use avr_device::interrupt::Mutex;
use arduino_hal::port::{mode, Pin};

pub(crate) const KEY_N: usize = 4;
pub(crate) const RELEASED: bool = false;
#[allow(dead_code)]
pub(crate) const PRESSED: bool = true;

pub(crate) const SW1: usize = 0;
pub(crate) const SW2: usize = 1;
pub(crate) const SW3: usize = 2;
pub(crate) const SW4: usize = 3;

const DEBOUNCE_MS: u32 = 30;
const LONG_PRESS_MS: u32 = 650;
const SW1_PINE_BIT: u8 = register_bit(4);
const SW2_PINE_BIT: u8 = register_bit(5);
const SW3_PIND_BIT: u8 = register_bit(3);
const SW4_PIND_BIT: u8 = register_bit(2);

static BUTTON_RUNTIME: Mutex<Cell<ButtonRuntime>> = Mutex::new(Cell::new(ButtonRuntime::new()));

#[derive(Copy, Clone, Eq, PartialEq)]
pub(crate) enum ButtonEvent {
    None,
    Short,
    Long,
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct ButtonMask(u8);

impl ButtonMask {
    const NONE: Self = Self(0);

    #[inline(always)]
    const fn from_key(key: usize) -> Self {
        Self(1u8 << key)
    }

    #[inline(always)]
    const fn contains(self, key: usize) -> bool {
        self.0 & Self::from_key(key).0 != 0
    }

    #[inline(always)]
    fn insert(&mut self, key: usize) {
        self.0 |= Self::from_key(key).0;
    }

    #[inline(always)]
    fn remove(&mut self, key: usize) {
        self.0 &= !Self::from_key(key).0;
    }

    #[inline(always)]
    fn take(&mut self) -> Self {
        let mask = *self;
        *self = Self::NONE;
        mask
    }
}

#[derive(Copy, Clone)]
struct ButtonRuntime {
    stable_mask: ButtonMask,
    debounce_pending_mask: ButtonMask,
    long_sent_mask: ButtonMask,
    short_event_mask: ButtonMask,
    long_event_mask: ButtonMask,
    debounce_due_ms: [u32; KEY_N],
    pressed_ms: [u32; KEY_N],
}

impl ButtonRuntime {
    const fn new() -> Self {
        Self {
            stable_mask: ButtonMask::NONE,
            debounce_pending_mask: ButtonMask::NONE,
            long_sent_mask: ButtonMask::NONE,
            short_event_mask: ButtonMask::NONE,
            long_event_mask: ButtonMask::NONE,
            debounce_due_ms: [0; KEY_N],
            pressed_ms: [0; KEY_N],
        }
    }

    const fn with_stable_mask(stable_mask: ButtonMask) -> Self {
        Self {
            stable_mask,
            debounce_pending_mask: ButtonMask::NONE,
            long_sent_mask: ButtonMask::NONE,
            short_event_mask: ButtonMask::NONE,
            long_event_mask: ButtonMask::NONE,
            debounce_due_ms: [0; KEY_N],
            pressed_ms: [0; KEY_N],
        }
    }

    fn schedule_debounce(&mut self, key: usize, now_ms: u32) {
        self.debounce_pending_mask.insert(key);
        self.debounce_due_ms[key] = now_ms + DEBOUNCE_MS;
    }

    fn settle_key(&mut self, key: usize, raw_mask: ButtonMask, now_ms: u32) {
        let is_pressed = raw_mask.contains(key);
        let was_pressed = self.stable_mask.contains(key);

        if is_pressed == was_pressed {
            return;
        }

        if is_pressed {
            self.stable_mask.insert(key);
            self.pressed_ms[key] = now_ms;
            self.long_sent_mask.remove(key);
        } else {
            self.stable_mask.remove(key);

            if !self.long_sent_mask.contains(key) {
                self.short_event_mask.insert(key);
            }

            self.long_sent_mask.remove(key);
        }
    }

    fn update_long_press(&mut self, key: usize, now_ms: u32) {
        if !self.stable_mask.contains(key) || self.long_sent_mask.contains(key) {
            return;
        }

        if now_ms - self.pressed_ms[key] >= LONG_PRESS_MS {
            self.long_sent_mask.insert(key);
            self.long_event_mask.insert(key);
        }
    }
}

#[allow(dead_code)]
pub(crate) const fn blank_state() -> [bool; KEY_N] {
    [RELEASED; KEY_N]
}

pub(crate) struct HsKey4b {
    keys: [Pin<mode::Input>; KEY_N],
}

#[allow(dead_code)]
impl HsKey4b {
    pub(crate) fn new(keys: [Pin<mode::Input>; KEY_N]) -> Self {
        Self { keys }
    }

    pub(crate) fn read(&self) -> [bool; KEY_N] {
        let keys = &self.keys;
        core::array::from_fn(|index| keys[index].is_low())
    }

    pub(crate) fn pressed(&self, index: usize) -> bool {
        self.keys[index].is_low()
    }

    pub(crate) fn any_pressed(&self) -> bool {
        self.keys.iter().any(|key| key.is_low())
    }

    pub(crate) fn read_mask(&self) -> u8 {
        let mut mask = 0u8;

        for (index, key) in self.keys.iter().enumerate() {
            if key.is_low() {
                mask |= 1 << index;
            }
        }

        mask
    }
}

pub(crate) fn init_runtime() {
    avr_device::interrupt::free(|cs| {
        BUTTON_RUNTIME
            .borrow(cs)
            .set(ButtonRuntime::with_stable_mask(read_button_mask()));
    });
}

pub(crate) fn schedule_debounce(key: usize, now_ms: u32) {
    avr_device::interrupt::free(|cs| {
        let cell = BUTTON_RUNTIME.borrow(cs);
        let mut state = cell.get();
        state.schedule_debounce(key, now_ms);
        cell.set(state);
    });
}

pub(crate) fn tick(now_ms: u32) {
    avr_device::interrupt::free(|cs| {
        let raw_mask = read_button_mask();
        let state_cell = BUTTON_RUNTIME.borrow(cs);
        let mut state = state_cell.get();

        for index in 0..KEY_N {
            if state.debounce_pending_mask.contains(index) && now_ms >= state.debounce_due_ms[index]
            {
                state.debounce_pending_mask.remove(index);
                state.settle_key(index, raw_mask, now_ms);
            }
        }

        for index in 0..KEY_N {
            state.update_long_press(index, now_ms);
        }

        state_cell.set(state);
    });
}

pub(crate) fn drain_events() -> [ButtonEvent; KEY_N] {
    avr_device::interrupt::free(|cs| {
        let cell = BUTTON_RUNTIME.borrow(cs);
        let mut state = cell.get();
        let short_mask = state.short_event_mask.take();
        let long_mask = state.long_event_mask.take();
        cell.set(state);

        core::array::from_fn(|index| {
            if long_mask.contains(index) {
                ButtonEvent::Long
            } else if short_mask.contains(index) {
                ButtonEvent::Short
            } else {
                ButtonEvent::None
            }
        })
    })
}

#[inline(always)]
const fn register_bit(bit: u8) -> u8 {
    1u8 << bit
}

#[inline(always)]
fn register_is_low(register_value: u8, bit: u8) -> bool {
    register_value & bit == 0
}

fn read_button_mask() -> ButtonMask {
    let portd = unsafe { &*arduino_hal::pac::PORTD::ptr() };
    let porte = unsafe { &*arduino_hal::pac::PORTE::ptr() };
    let pind = portd.pind().read().bits();
    let pine = porte.pine().read().bits();
    let mut mask = ButtonMask::NONE;

    if register_is_low(pine, SW1_PINE_BIT) {
        mask.insert(SW1);
    }
    if register_is_low(pine, SW2_PINE_BIT) {
        mask.insert(SW2);
    }
    if register_is_low(pind, SW3_PIND_BIT) {
        mask.insert(SW3);
    }
    if register_is_low(pind, SW4_PIND_BIT) {
        mask.insert(SW4);
    }

    mask
}
