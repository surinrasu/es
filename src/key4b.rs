use arduino_hal::port::{mode, Pin};

pub(crate) const KEY_N: usize = 4;
pub(crate) const RELEASED: bool = false;
pub(crate) const PRESSED: bool = true;

pub(crate) const SW1: usize = 0;
pub(crate) const SW2: usize = 1;
pub(crate) const SW3: usize = 2;
pub(crate) const SW4: usize = 3;

pub(crate) const fn blank_state() -> [bool; KEY_N] {
    [RELEASED; KEY_N]
}

pub(crate) struct HsKey4b {
    keys: [Pin<mode::Input>; KEY_N],
}

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
            // The HS-KEY4B silkscreen states "Press output low".
            if key.is_low() {
                mask |= 1 << index;
            }
        }

        mask
    }
}
