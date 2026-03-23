use core::{arch::asm, convert::Infallible};

use arduino_hal::port::{mode, Pin, PinOps};
use smart_leds::{RGB8, SmartLedsWrite};

pub(crate) const LED_N: usize = 12;
pub(crate) const OFF: RGB8 = RGB8 { r: 0, g: 0, b: 0 };

const SOFT_MAX_INTENSITY: f32 = 48.0;
const LAB_DELTA: f32 = 6.0 / 29.0;
const D65_X: f32 = 0.95047;
const D65_Y: f32 = 1.0;
const D65_Z: f32 = 1.08883;
const PI: f32 = core::f32::consts::PI;
const HALF_PI: f32 = core::f32::consts::FRAC_PI_2;
const TAU: f32 = PI * 2.0;
const MEGA2560_D51_PORT_IO: u8 = 0x05;
const MEGA2560_D51_PIN_BIT: u8 = 2;
#[es_sim::export]
pub(crate) const LED_DATA_PIN: es_sim::DigitalPin = es_sim::DigitalPin::new('B', 2);
const LATCH_DELAY_US: u32 = 80;

struct Lab {
    lightness: f32,
    a: f32,
    b: f32,
}

struct Xyz {
    x: f32,
    y: f32,
    z: f32,
}

struct LinearRgb {
    r: f32,
    g: f32,
    b: f32,
}

pub(crate) const fn blank_frame() -> [RGB8; LED_N] {
    [OFF; LED_N]
}

pub(crate) struct HsF12a<PIN> {
    leds: Mega2560Ws2812<PIN>,
}

impl<PIN: PinOps> HsF12a<PIN> {
    pub(crate) fn new(data: Pin<mode::Output, PIN>) -> Self {
        Self {
            leds: Mega2560Ws2812::new(data),
        }
    }

    pub(crate) fn write(&mut self, frame: &[RGB8; LED_N]) {
        let _ = self.leds.write(frame.iter().copied());
    }
}

struct Mega2560Ws2812<PIN> {
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

pub(crate) fn fill_spaced_palette<const N: usize>(
    colors: &mut [RGB8; N],
    base_hue: f32,
    lightness: f32,
    chroma: f32,
    hue_step_degrees: f32,
) {
    for (slot, color) in colors.iter_mut().enumerate() {
        let hue = base_hue + slot as f32 * hue_step_degrees;
        *color = lch_to_rgb(lightness, chroma, hue);
    }
}

pub(crate) fn lch_to_rgb(lightness: f32, chroma: f32, hue_degrees: f32) -> RGB8 {
    let lab = lch_to_lab(lightness, chroma, hue_degrees);
    let xyz = lab_to_xyz(lab);
    let linear = xyz_to_linear_rgb(xyz);
    linear_rgb_to_rgb8(linear)
}

fn lch_to_lab(lightness: f32, chroma: f32, hue_degrees: f32) -> Lab {
    let hue_radians = hue_degrees * PI / 180.0;

    Lab {
        lightness,
        a: chroma * cos_approx(hue_radians),
        b: chroma * sin_approx(hue_radians),
    }
}

fn lab_to_xyz(lab: Lab) -> Xyz {
    let fy = (lab.lightness + 16.0) / 116.0;
    let fx = fy + lab.a / 500.0;
    let fz = fy - lab.b / 200.0;

    Xyz {
        x: D65_X * lab_inverse(fx),
        y: D65_Y * lab_inverse(fy),
        z: D65_Z * lab_inverse(fz),
    }
}

fn xyz_to_linear_rgb(xyz: Xyz) -> LinearRgb {
    LinearRgb {
        r: 3.2406 * xyz.x - 1.5372 * xyz.y - 0.4986 * xyz.z,
        g: -0.9689 * xyz.x + 1.8758 * xyz.y + 0.0415 * xyz.z,
        b: 0.0557 * xyz.x - 0.2040 * xyz.y + 1.0570 * xyz.z,
    }
}

fn linear_rgb_to_rgb8(rgb: LinearRgb) -> RGB8 {
    RGB8 {
        r: scale_channel(rgb.r),
        g: scale_channel(rgb.g),
        b: scale_channel(rgb.b),
    }
}

fn lab_inverse(t: f32) -> f32 {
    if t > LAB_DELTA {
        t * t * t
    } else {
        3.0 * LAB_DELTA * LAB_DELTA * (t - 4.0 / 29.0)
    }
}

fn scale_channel(value: f32) -> u8 {
    let clamped = clamp_unit(value);
    (clamped * SOFT_MAX_INTENSITY + 0.5) as u8
}

fn clamp_unit(value: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

fn sin_approx(angle: f32) -> f32 {
    let wrapped = wrap_radians(angle);
    let y = 1.273_239_5 * wrapped - 0.405_284_73 * wrapped * abs_f32(wrapped);
    0.225 * (y * abs_f32(y) - y) + y
}

fn cos_approx(angle: f32) -> f32 {
    sin_approx(angle + HALF_PI)
}

fn wrap_radians(mut angle: f32) -> f32 {
    while angle > PI {
        angle -= TAU;
    }

    while angle < -PI {
        angle += TAU;
    }

    angle
}

fn abs_f32(value: f32) -> f32 {
    if value < 0.0 { -value } else { value }
}
