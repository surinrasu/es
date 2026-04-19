use core::cell::Cell;

use arduino_hal::{
    pac,
    port::{mode, PinOps},
};
use embedded_hal::i2c::I2c;

const STOP_CM: u16 = 25;
const CAUTION_CM: u16 = 50;
const NOTICE_CM: u16 = 80;

const TIMER0_PRESCALER: u32 = 64;
const TIMER0_COUNTS: u8 = 250;
const MILLIS_INCREMENT: u32 = 1;
const MEASURE_INTERVAL_MS: u32 = 50;
const LOOP_TICK_MS: u32 = 20;
const VALID_READING_HOLD_MS: u32 = 300;
const SENSOR_TIMEOUT_US: u32 = 15_000;
const MAX_VALID_DISTANCE_CM: u16 = 199;
const I2C_SPEED_HZ: u32 = 400_000;

static MILLIS_COUNTER: avr_device::interrupt::Mutex<Cell<u32>> =
    avr_device::interrupt::Mutex::new(Cell::new(0));

#[derive(Copy, Clone, Eq, PartialEq)]
enum AlertBand {
    Clear,
    Notice,
    Caution,
    Stop,
}

impl AlertBand {
    const fn from_distance(distance_cm: Option<u16>) -> Self {
        match distance_cm {
            Some(distance_cm) if distance_cm <= STOP_CM => Self::Stop,
            Some(distance_cm) if distance_cm <= CAUTION_CM => Self::Caution,
            Some(distance_cm) if distance_cm <= NOTICE_CM => Self::Notice,
            _ => Self::Clear,
        }
    }

    const fn active_layer(self) -> usize {
        match self {
            Self::Clear => 0,
            Self::Notice => 1,
            Self::Caution => 2,
            Self::Stop => 3,
        }
    }

    const fn cadence(self) -> Option<Cadence> {
        match self {
            Self::Clear => None,
            Self::Notice => Some(Cadence {
                on_ms: 100,
                off_ms: 500,
                tone_hz: 1_500,
            }),
            Self::Caution => Some(Cadence {
                on_ms: 100,
                off_ms: 250,
                tone_hz: 1_850,
            }),
            Self::Stop => Some(Cadence {
                on_ms: 100,
                off_ms: 100,
                tone_hz: 2_250,
            }),
        }
    }

    fn blink_on(self, now_ms: u32) -> bool {
        match self.cadence() {
            Some(cadence) => cadence.phase_on(now_ms),
            None => true,
        }
    }
}

#[derive(Copy, Clone)]
struct Cadence {
    on_ms: u32,
    off_ms: u32,
    tone_hz: u16,
}

impl Cadence {
    const fn cycle_ms(self) -> u32 {
        self.on_ms + self.off_ms
    }

    fn phase_on(self, now_ms: u32) -> bool {
        now_ms % self.cycle_ms() < self.on_ms
    }
}

struct DistanceTracker {
    filtered_cm: Option<u16>,
    last_valid_ms: u32,
}

impl DistanceTracker {
    const fn new() -> Self {
        Self {
            filtered_cm: None,
            last_valid_ms: 0,
        }
    }

    fn update(&mut self, sample_cm: Option<u16>, now_ms: u32) {
        if let Some(sample_cm) = sample_cm {
            self.filtered_cm = Some(match self.filtered_cm {
                Some(previous_cm) => smooth_distance(previous_cm, sample_cm),
                None => sample_cm,
            });
            self.last_valid_ms = now_ms;
            return;
        }

        if now_ms.wrapping_sub(self.last_valid_ms) > VALID_READING_HOLD_MS {
            self.filtered_cm = None;
        }
    }

    const fn current(&self) -> Option<u16> {
        self.filtered_cm
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct RenderState {
    band: AlertBand,
    blink_on: bool,
    display_cm: Option<u16>,
}

#[derive(Copy, Clone)]
struct Point {
    x: i16,
    y: i16,
}

#[derive(Copy, Clone)]
struct RadarLayer {
    top_left: Point,
    top_right: Point,
    bottom_left: Point,
    bottom_right: Point,
}

const RADAR_LAYERS: [RadarLayer; 4] = [
    RadarLayer {
        top_left: Point { x: 10, y: 11 },
        top_right: Point { x: 69, y: 11 },
        bottom_left: Point { x: 2, y: 33 },
        bottom_right: Point { x: 77, y: 33 },
    },
    RadarLayer {
        top_left: Point { x: 18, y: 17 },
        top_right: Point { x: 61, y: 17 },
        bottom_left: Point { x: 12, y: 36 },
        bottom_right: Point { x: 67, y: 36 },
    },
    RadarLayer {
        top_left: Point { x: 26, y: 24 },
        top_right: Point { x: 53, y: 24 },
        bottom_left: Point { x: 22, y: 39 },
        bottom_right: Point { x: 57, y: 39 },
    },
    RadarLayer {
        top_left: Point { x: 31, y: 31 },
        top_right: Point { x: 48, y: 31 },
        bottom_left: Point { x: 29, y: 42 },
        bottom_right: Point { x: 50, y: 42 },
    },
];

const SEGMENT_A: u8 = 1 << 0;
const SEGMENT_B: u8 = 1 << 1;
const SEGMENT_C: u8 = 1 << 2;
const SEGMENT_D: u8 = 1 << 3;
const SEGMENT_E: u8 = 1 << 4;
const SEGMENT_F: u8 = 1 << 5;
const SEGMENT_G: u8 = 1 << 6;

const DIGIT_SEGMENTS: [u8; 10] = [
    SEGMENT_A | SEGMENT_B | SEGMENT_C | SEGMENT_D | SEGMENT_E | SEGMENT_F,
    SEGMENT_B | SEGMENT_C,
    SEGMENT_A | SEGMENT_B | SEGMENT_D | SEGMENT_E | SEGMENT_G,
    SEGMENT_A | SEGMENT_B | SEGMENT_C | SEGMENT_D | SEGMENT_G,
    SEGMENT_B | SEGMENT_C | SEGMENT_F | SEGMENT_G,
    SEGMENT_A | SEGMENT_C | SEGMENT_D | SEGMENT_F | SEGMENT_G,
    SEGMENT_A | SEGMENT_C | SEGMENT_D | SEGMENT_E | SEGMENT_F | SEGMENT_G,
    SEGMENT_A | SEGMENT_B | SEGMENT_C,
    SEGMENT_A | SEGMENT_B | SEGMENT_C | SEGMENT_D | SEGMENT_E | SEGMENT_F | SEGMENT_G,
    SEGMENT_A | SEGMENT_B | SEGMENT_C | SEGMENT_D | SEGMENT_F | SEGMENT_G,
];

fn millis_init(tc0: pac::TC0) {
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

#[avr_device::interrupt(atmega2560)]
fn TIMER0_COMPA() {
    avr_device::interrupt::free(|cs| {
        let millis_cell = MILLIS_COUNTER.borrow(cs);
        let now_ms = millis_cell.get().wrapping_add(MILLIS_INCREMENT);
        millis_cell.set(now_ms);
    });
}

pub(crate) fn run() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    let mut buzzer = crate::f02a::HsF02a::new(pins.d4.into_output(), dp.TC2);
    let mut sensor =
        crate::sr04::HsSr04::new(pins.d8.into_output(), pins.d9.into_floating_input(), dp.TC1);
    sensor.set_timeout_us(SENSOR_TIMEOUT_US);

    let i2c = arduino_hal::I2c::new(
        dp.TWI,
        pins.d20.into_pull_up_input(),
        pins.d21.into_pull_up_input(),
        I2C_SPEED_HZ,
    );
    let mut oled = crate::ssd1315::HsOledI2c::new(i2c);
    let _ = oled.init();

    millis_init(dp.TC0);
    unsafe {
        avr_device::interrupt::enable();
    }

    let mut tracker = DistanceTracker::new();
    let mut last_measure_ms = 0;
    let mut rendered_state: Option<RenderState> = None;

    loop {
        let now_ms = millis();
        buzzer.poll(now_ms);

        if now_ms.wrapping_sub(last_measure_ms) >= MEASURE_INTERVAL_MS {
            let sample_cm = read_distance_cm(&mut sensor);
            tracker.update(sample_cm, now_ms);
            last_measure_ms = now_ms;
        }

        let display_cm = tracker
            .current()
            .map(|distance_cm| distance_cm.min(MAX_VALID_DISTANCE_CM));
        let band = AlertBand::from_distance(display_cm);
        let blink_on = band.blink_on(now_ms);
        let state = RenderState {
            band,
            blink_on,
            display_cm,
        };

        if rendered_state != Some(state) {
            render_dashboard(&mut oled, state);
            rendered_state = Some(state);
        }

        match band.cadence() {
            Some(cadence) if blink_on => buzzer.play_tone(cadence.tone_hz, LOOP_TICK_MS, now_ms),
            _ => {
                buzzer.silence();
                arduino_hal::delay_ms(LOOP_TICK_MS);
            }
        }
    }
}

fn read_distance_cm<TRIG, ECHO, IMODE>(
    sensor: &mut crate::sr04::HsSr04<TRIG, ECHO, IMODE>,
) -> Option<u16>
where
    TRIG: PinOps,
    ECHO: PinOps,
    IMODE: mode::InputMode,
{
    match sensor.measure() {
        Ok(reading) => {
            let distance_cm = reading.distance_cm();
            if (2..=MAX_VALID_DISTANCE_CM).contains(&distance_cm) {
                Some(distance_cm)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

fn smooth_distance(previous_cm: u16, sample_cm: u16) -> u16 {
    let delta = if previous_cm > sample_cm {
        previous_cm - sample_cm
    } else {
        sample_cm - previous_cm
    };

    if delta >= 18 {
        sample_cm
    } else {
        ((previous_cm as u32 * 2 + sample_cm as u32 + 1) / 3) as u16
    }
}

fn render_dashboard<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, state: RenderState)
where
    I2C: I2c,
{
    oled.clear();

    draw_vline(oled, 82, 6, 58, true);
    draw_radar_panel(oled, state.band, state.blink_on);
    draw_distance_panel(oled, state.display_cm, state.band, state.blink_on);

    let _ = oled.flush();
}

fn draw_radar_panel<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, band: AlertBand, blink_on: bool)
where
    I2C: I2c,
{
    let active_layer = band.active_layer();

    for (index, layer) in RADAR_LAYERS.iter().enumerate() {
        let visible = if index < active_layer {
            true
        } else if index == active_layer {
            blink_on
        } else {
            false
        };

        if visible {
            draw_radar_layer(oled, *layer);
        }
    }

    draw_car_icon(oled, band == AlertBand::Stop && blink_on);
}

fn draw_distance_panel<I2C>(
    oled: &mut crate::ssd1315::HsOledI2c<I2C>,
    display_cm: Option<u16>,
    band: AlertBand,
    blink_on: bool,
) where
    I2C: I2c,
{
    let x_positions = [88, 102, 116];
    let y = 12;
    let digits = distance_to_digits(display_cm);

    for (&x, digit) in x_positions.iter().zip(digits.iter()) {
        match digit {
            Some(digit) => draw_seven_segment_digit(oled, x, y, *digit),
            None => draw_seven_segment_dash(oled, x, y),
        }
    }

    draw_glyph(oled, 102, 46, &GLYPH_C);
    draw_glyph(oled, 109, 46, &GLYPH_M);
    draw_alert_meter(oled, band, blink_on);
}

fn draw_radar_layer<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, layer: RadarLayer)
where
    I2C: I2c,
{
    draw_dashed_line(oled, layer.top_left, layer.top_right, 7, 4, true);
    draw_dashed_line(oled, layer.top_left, layer.bottom_left, 6, 3, true);
    draw_dashed_line(oled, layer.top_right, layer.bottom_right, 6, 3, true);
}

fn draw_car_icon<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, cabin_flash: bool)
where
    I2C: I2c,
{
    draw_rect(oled, 28, 47, 21, 11, true);
    draw_rect(oled, 33, 43, 11, 5, true);
    draw_hline(oled, 27, 29, 4, true);
    draw_hline(oled, 49, 51, 4, true);
    draw_hline(oled, 27, 29, 56, true);
    draw_hline(oled, 49, 51, 56, true);

    if cabin_flash {
        fill_rect(oled, 35, 45, 7, 2, true);
    }
}

fn distance_to_digits(distance_cm: Option<u16>) -> [Option<u8>; 3] {
    match distance_cm {
        Some(distance_cm) => {
            let hundreds = distance_cm / 100;
            let tens = (distance_cm / 10) % 10;
            let ones = distance_cm % 10;

            [
                if hundreds > 0 {
                    Some(hundreds as u8)
                } else {
                    None
                },
                if hundreds > 0 || tens > 0 {
                    Some(tens as u8)
                } else {
                    None
                },
                Some(ones as u8),
            ]
        }
        None => [None, None, None],
    }
}

fn draw_seven_segment_digit<I2C>(
    oled: &mut crate::ssd1315::HsOledI2c<I2C>,
    x: i16,
    y: i16,
    digit: u8,
) where
    I2C: I2c,
{
    let segments = DIGIT_SEGMENTS[digit as usize];
    draw_seven_segment_pattern(oled, x, y, segments);
}

fn draw_seven_segment_dash<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, x: i16, y: i16)
where
    I2C: I2c,
{
    draw_seven_segment_pattern(oled, x, y, SEGMENT_G);
}

fn draw_seven_segment_pattern<I2C>(
    oled: &mut crate::ssd1315::HsOledI2c<I2C>,
    x: i16,
    y: i16,
    segments: u8,
) where
    I2C: I2c,
{
    const WIDTH: i16 = 11;
    const HEIGHT: i16 = 21;
    const THICKNESS: i16 = 2;
    let mid_y = y + HEIGHT / 2;
    let bottom_y = y + HEIGHT - THICKNESS;
    let right_x = x + WIDTH - THICKNESS;
    let lower_height = HEIGHT / 2 - THICKNESS;

    if segments & SEGMENT_A != 0 {
        fill_rect(
            oled,
            x + THICKNESS,
            y,
            WIDTH - THICKNESS * 2,
            THICKNESS,
            true,
        );
    }
    if segments & SEGMENT_B != 0 {
        fill_rect(oled, right_x, y + THICKNESS, THICKNESS, lower_height, true);
    }
    if segments & SEGMENT_C != 0 {
        fill_rect(oled, right_x, mid_y, THICKNESS, lower_height, true);
    }
    if segments & SEGMENT_D != 0 {
        fill_rect(
            oled,
            x + THICKNESS,
            bottom_y,
            WIDTH - THICKNESS * 2,
            THICKNESS,
            true,
        );
    }
    if segments & SEGMENT_E != 0 {
        fill_rect(oled, x, mid_y, THICKNESS, lower_height, true);
    }
    if segments & SEGMENT_F != 0 {
        fill_rect(oled, x, y + THICKNESS, THICKNESS, lower_height, true);
    }
    if segments & SEGMENT_G != 0 {
        fill_rect(
            oled,
            x + THICKNESS,
            mid_y - THICKNESS / 2,
            WIDTH - THICKNESS * 2,
            THICKNESS,
            true,
        );
    }
}

const GLYPH_C: [u8; 5] = [0b0011110, 0b0100001, 0b0100001, 0b0100001, 0b0010010];
const GLYPH_M: [u8; 5] = [0b0111111, 0b0100000, 0b0010000, 0b0100000, 0b0111111];

fn draw_alert_meter<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, band: AlertBand, blink_on: bool)
where
    I2C: I2c,
{
    let level = band.active_layer() as i16;

    for slot in 0..3 {
        let x = 88 + slot * 12;
        let active = slot < level;
        let blinking = slot + 1 == level && band != AlertBand::Clear;
        let filled = active && (!blinking || blink_on);

        draw_rect(oled, x, 56, 8, 5, true);
        if filled {
            fill_rect(oled, x + 1, 57, 6, 3, true);
        }
    }

    if band == AlertBand::Stop && blink_on {
        draw_glyph(oled, 123, 54, &GLYPH_BANG);
    }
}

const GLYPH_BANG: [u8; 3] = [0b1111101, 0b0000000, 0b1111101];

fn draw_glyph<I2C, const N: usize>(
    oled: &mut crate::ssd1315::HsOledI2c<I2C>,
    x: i16,
    y: i16,
    glyph: &[u8; N],
) where
    I2C: I2c,
{
    for (column, bits) in glyph.iter().enumerate() {
        for row in 0..7 {
            if bits & (1 << (6 - row)) != 0 {
                set_pixel(oled, x + column as i16, y + row as i16, true);
            }
        }
    }
}

fn draw_rect<I2C>(
    oled: &mut crate::ssd1315::HsOledI2c<I2C>,
    x: i16,
    y: i16,
    w: i16,
    h: i16,
    on: bool,
) where
    I2C: I2c,
{
    if w <= 0 || h <= 0 {
        return;
    }

    draw_hline(oled, x, x + w - 1, y, on);
    draw_hline(oled, x, x + w - 1, y + h - 1, on);
    draw_vline(oled, x, y, y + h - 1, on);
    draw_vline(oled, x + w - 1, y, y + h - 1, on);
}

fn fill_rect<I2C>(
    oled: &mut crate::ssd1315::HsOledI2c<I2C>,
    x: i16,
    y: i16,
    w: i16,
    h: i16,
    on: bool,
) where
    I2C: I2c,
{
    if w <= 0 || h <= 0 {
        return;
    }

    for yy in y..(y + h) {
        draw_hline(oled, x, x + w - 1, yy, on);
    }
}

fn draw_hline<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, x0: i16, x1: i16, y: i16, on: bool)
where
    I2C: I2c,
{
    let start = x0.min(x1);
    let end = x0.max(x1);

    for x in start..=end {
        set_pixel(oled, x, y, on);
    }
}

fn draw_vline<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, x: i16, y0: i16, y1: i16, on: bool)
where
    I2C: I2c,
{
    let start = y0.min(y1);
    let end = y0.max(y1);

    for y in start..=end {
        set_pixel(oled, x, y, on);
    }
}

fn draw_dashed_line<I2C>(
    oled: &mut crate::ssd1315::HsOledI2c<I2C>,
    from: Point,
    to: Point,
    dash_on: i16,
    dash_off: i16,
    on: bool,
) where
    I2C: I2c,
{
    let mut x = from.x;
    let mut y = from.y;
    let dx = abs_i16(to.x - from.x);
    let sx = if from.x < to.x { 1 } else { -1 };
    let dy = -abs_i16(to.y - from.y);
    let sy = if from.y < to.y { 1 } else { -1 };
    let mut error = dx + dy;
    let mut step = 0;
    let cycle = dash_on + dash_off;

    loop {
        if cycle == 0 || step % cycle < dash_on {
            set_pixel(oled, x, y, on);
        }

        if x == to.x && y == to.y {
            break;
        }

        let doubled_error = error * 2;
        if doubled_error >= dy {
            error += dy;
            x += sx;
        }
        if doubled_error <= dx {
            error += dx;
            y += sy;
        }
        step += 1;
    }
}

fn set_pixel<I2C>(oled: &mut crate::ssd1315::HsOledI2c<I2C>, x: i16, y: i16, on: bool)
where
    I2C: I2c,
{
    if x < 0 || y < 0 {
        return;
    }

    oled.set_pixel(x as usize, y as usize, on);
}

const fn abs_i16(value: i16) -> i16 {
    if value < 0 {
        -value
    } else {
        value
    }
}
