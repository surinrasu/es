use smart_leds::RGB8;

pub(crate) const LED_N: usize = 12;
pub(crate) const OFF: RGB8 = RGB8 { r: 0, g: 0, b: 0 };

const RNG_FALLBACK_SEED: u32 = 0xC0DE_2560;
const SOFT_MAX_INTENSITY: f32 = 48.0;
const LAB_DELTA: f32 = 6.0 / 29.0;
const D65_X: f32 = 0.95047;
const D65_Y: f32 = 1.0;
const D65_Z: f32 = 1.08883;
const PI: f32 = core::f32::consts::PI;
const HALF_PI: f32 = core::f32::consts::FRAC_PI_2;
const TAU: f32 = PI * 2.0;

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

    pub(crate) fn next_unit_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
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

pub(crate) fn fill_spaced_palette<const N: usize>(
    rng: &mut XorShift32,
    colors: &mut [RGB8; N],
    lightness: f32,
    chroma: f32,
    hue_step_degrees: f32,
) {
    let base_hue = rng.next_unit_f32() * 360.0;

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

pub(crate) fn render_phase(
    frame: &mut [RGB8; LED_N],
    active_leds: &[usize],
    active_colors: &[RGB8],
    phase: usize,
) {
    frame.fill(OFF);

    for (slot, (&led, &color)) in active_leds.iter().zip(active_colors.iter()).enumerate() {
        if slot % 2 == phase % 2 {
            frame[led] = color;
        }
    }
}
