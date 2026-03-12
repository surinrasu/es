const BEAT_DELAY: u32 = 220;
const CYCLE_GAP: u32 = 80;
const BEAT_N: usize = 16;
const ACTIVE_N: usize = 4;

pub(crate) fn run() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    let mut panel = crate::f03a::HsF03a::new([
        // Leaving D0/D1 free for serial upload/debug
        pins.d2.into_output().downgrade(),
        pins.d3.into_output().downgrade(),
        pins.d4.into_output().downgrade(),
        pins.d5.into_output().downgrade(),
        pins.d6.into_output().downgrade(),
        pins.d7.into_output().downgrade(),
        pins.d8.into_output().downgrade(),
        pins.d9.into_output().downgrade(),
        pins.d10.into_output().downgrade(),
        pins.d11.into_output().downgrade(),
        pins.d12.into_output().downgrade(),
        pins.d13.into_output().downgrade(),
    ]);
    let mut rng = crate::f03a::XorShift32::new(crate::f03a::seed_rng());
    let mut frame = crate::f03a::blank_frame();
    let mut active_leds = [0usize; ACTIVE_N];

    panel.write(&frame);
    arduino_hal::delay_ms(100);

    loop {
        crate::f03a::pick_distinct_leds(&mut rng, &mut active_leds);

        for beat in 0..BEAT_N {
            crate::f03a::render_phase(&mut frame, &active_leds, beat);
            panel.write(&frame);
            arduino_hal::delay_ms(BEAT_DELAY);
        }

        frame.fill(crate::f03a::OFF);
        panel.write(&frame);
        arduino_hal::delay_ms(CYCLE_GAP);
    }
}
