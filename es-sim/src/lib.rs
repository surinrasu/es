#![cfg_attr(not(feature = "std"), no_std)]

extern crate self as es_sim;

pub use es_sim_macros::{export, test};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct DigitalPin {
    port: char,
    pin: u8,
}

impl DigitalPin {
    pub const fn new(port: char, pin: u8) -> Self {
        Self { port, pin }
    }

    pub const fn port(self) -> char {
        self.port
    }

    pub const fn pin(self) -> u8 {
        self.pin
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PinTransition {
    pub elapsed_cycles: u64,
    pub level: bool,
}

#[cfg(feature = "std")]
mod runtime {
    use std::env;
    use std::path::{Path, PathBuf};

    use avr_simulator::{AvrSimulator, AvrState};

    use crate::{DigitalPin, PinTransition};

    const DEFAULT_MCU: &str = "atmega2560";
    const DEFAULT_CLOCK_HZ: u32 = 16_000_000;

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub struct CaseConfig {
        pub timeout_ms: u64,
        pub boot_ms: u64,
        pub reset: bool,
    }

    impl CaseConfig {
        pub const DEFAULT_TIMEOUT_MS: u64 = 1_000;
        pub const DEFAULT_BOOT_MS: u64 = 0;
        pub const DEFAULT_RESET: bool = true;
    }

    impl Default for CaseConfig {
        fn default() -> Self {
            Self {
                timeout_ms: Self::DEFAULT_TIMEOUT_MS,
                boot_ms: Self::DEFAULT_BOOT_MS,
                reset: Self::DEFAULT_RESET,
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct CaseMetadata {
        entry: &'static str,
        case_name: &'static str,
        project_dir: PathBuf,
        elf_path: PathBuf,
        config: CaseConfig,
    }

    impl CaseMetadata {
        pub fn new(
            entry: &'static str,
            case_name: &'static str,
            config: CaseConfig,
            project_dir: PathBuf,
            elf_path: PathBuf,
        ) -> Self {
            Self {
                entry,
                case_name,
                project_dir,
                elf_path,
                config,
            }
        }

        pub fn entry(&self) -> &'static str {
            self.entry
        }

        pub fn case_name(&self) -> &'static str {
            self.case_name
        }

        pub fn project_dir(&self) -> &Path {
            &self.project_dir
        }

        pub fn elf_path(&self) -> &Path {
            &self.elf_path
        }

        pub fn config(&self) -> CaseConfig {
            self.config
        }

        pub fn mcu(&self) -> &'static str {
            DEFAULT_MCU
        }

        pub fn clock_hz(&self) -> u32 {
            DEFAULT_CLOCK_HZ
        }
    }

    #[derive(Debug)]
    pub struct Sim {
        metadata: CaseMetadata,
        simulator: AvrSimulator,
        elapsed_cycles: u64,
        booted: bool,
        last_state: Option<AvrState>,
    }

    impl Sim {
        pub fn new(metadata: CaseMetadata) -> Self {
            let simulator = Self::spawn_simulator(&metadata);

            Self {
                metadata,
                simulator,
                elapsed_cycles: 0,
                booted: false,
                last_state: None,
            }
        }

        pub fn metadata(&self) -> &CaseMetadata {
            &self.metadata
        }

        pub fn config(&self) -> CaseConfig {
            self.metadata.config()
        }

        pub fn entry(&self) -> &'static str {
            self.metadata.entry()
        }

        pub fn case_name(&self) -> &'static str {
            self.metadata.case_name()
        }

        pub fn project_dir(&self) -> &Path {
            self.metadata.project_dir()
        }

        pub fn elf_path(&self) -> &Path {
            self.metadata.elf_path()
        }

        pub fn elapsed_cycles(&self) -> u64 {
            self.elapsed_cycles
        }

        pub fn elapsed_us(&self) -> u64 {
            scale_div(self.elapsed_cycles, 1_000_000, self.metadata.clock_hz() as u64)
        }

        pub fn elapsed_ms(&self) -> u64 {
            scale_div(self.elapsed_cycles, 1_000, self.metadata.clock_hz() as u64)
        }

        pub fn is_booted(&self) -> bool {
            self.booted
        }

        pub fn boot(&mut self) {
            if self.booted {
                return;
            }

            self.booted = true;
            self.advance_ms(self.metadata.config().boot_ms);
        }

        pub fn advance_ms(&mut self, delta_ms: u64) {
            self.advance_cycles(self.cycles_from_millis(delta_ms));
        }

        pub fn advance_us(&mut self, delta_us: u64) {
            self.advance_cycles(self.cycles_from_micros(delta_us));
        }

        pub fn advance_cycles(&mut self, delta_cycles: u64) {
            let deadline = self.elapsed_cycles.saturating_add(delta_cycles);

            while self.elapsed_cycles < deadline {
                self.step_once();
            }
        }

        pub fn set_pin(&mut self, pin: DigitalPin, high: bool) {
            self.simulator
                .set_digital_pin(pin.port(), pin.pin(), high);
        }

        pub fn digital_pin(&mut self, pin: DigitalPin) -> bool {
            self.simulator
                .get_digital_pin(pin.port(), pin.pin())
        }

        pub fn count_pin_edges(&mut self, pin: DigitalPin, duration_us: u64) -> usize {
            let deadline = self.elapsed_cycles.saturating_add(self.cycles_from_micros(duration_us));
            let mut edges = 0usize;
            let mut previous = self.digital_pin(pin);

            while self.elapsed_cycles < deadline {
                self.step_once();
                let current = self.digital_pin(pin);

                if current != previous {
                    edges += 1;
                    previous = current;
                }
            }

            edges
        }

        pub fn capture_pin_transitions(
            &mut self,
            pin: DigitalPin,
            duration_us: u64,
            max_transitions: usize,
        ) -> Vec<PinTransition> {
            let deadline = self.elapsed_cycles.saturating_add(self.cycles_from_micros(duration_us));
            let mut transitions = Vec::new();
            let mut previous = self.digital_pin(pin);

            while self.elapsed_cycles < deadline {
                self.step_once();
                let current = self.digital_pin(pin);

                if current != previous {
                    transitions.push(PinTransition {
                        elapsed_cycles: self.elapsed_cycles,
                        level: current,
                    });
                    previous = current;

                    if transitions.len() >= max_transitions {
                        break;
                    }
                }
            }

            transitions
        }

        pub fn reset(&mut self) {
            self.simulator = Self::spawn_simulator(&self.metadata);
            self.elapsed_cycles = 0;
            self.booted = false;
            self.last_state = None;
        }

        fn spawn_simulator(metadata: &CaseMetadata) -> AvrSimulator {
            AvrSimulator::new(metadata.mcu(), metadata.clock_hz(), metadata.elf_path())
        }

        fn cycles_from_millis(&self, millis: u64) -> u64 {
            scale_mul_div(millis, self.metadata.clock_hz() as u64, 1_000)
        }

        fn cycles_from_micros(&self, micros: u64) -> u64 {
            scale_mul_div(micros, self.metadata.clock_hz() as u64, 1_000_000)
        }

        fn step_once(&mut self) {
            let step = self.simulator.step();
            self.elapsed_cycles = self.elapsed_cycles.saturating_add(step.tt.as_cycles());
            self.last_state = Some(step.state);
            self.assert_within_timeout();

            match step.state {
                AvrState::Running | AvrState::Sleeping | AvrState::Step | AvrState::StepDone => {}
                AvrState::Crashed => {
                    panic!(
                        "AVR crashed in `{}` after {} cycles",
                        self.case_name(),
                        self.elapsed_cycles
                    );
                }
                AvrState::Done => {
                    panic!(
                        "AVR stopped in `{}` after {} cycles",
                        self.case_name(),
                        self.elapsed_cycles
                    );
                }
                state => {
                    panic!(
                        "unexpected AVR state {:?} in `{}` after {} cycles",
                        state,
                        self.case_name(),
                        self.elapsed_cycles
                    );
                }
            }
        }

        fn assert_within_timeout(&self) {
            let timeout_cycles =
                scale_mul_div(self.metadata.config().timeout_ms, self.metadata.clock_hz() as u64, 1_000);

            if self.elapsed_cycles > timeout_cycles {
                panic!(
                    "sim test `{}` timed out after {} ms (limit: {} ms)",
                    self.case_name(),
                    self.elapsed_ms(),
                    self.metadata.config().timeout_ms
                );
            }
        }
    }

    pub use CaseConfig as PublicCaseConfig;
    pub use CaseMetadata as PublicCaseMetadata;
    pub use Sim as PublicSim;

    #[allow(dead_code)]
    pub fn run_case(
        entry: &'static str,
        case_name: &'static str,
        config: CaseConfig,
        test_fn: fn(&mut Sim),
    ) {
        let project_dir = project_dir();
        let mut sim = Sim::new(CaseMetadata::new(
            entry,
            case_name,
            config,
            project_dir.clone(),
            resolve_elf_path(&project_dir),
        ));

        if config.reset {
            sim.reset();
        }

        test_fn(&mut sim);
    }

    #[allow(dead_code)]
    fn project_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("es-sim should live under the project root")
            .to_path_buf()
    }

    #[allow(dead_code)]
    fn resolve_elf_path(project_dir: &Path) -> PathBuf {
        if let Ok(path) = env::var("ES_SIM_ELF") {
            return PathBuf::from(path);
        }

        project_dir
            .join("target")
            .join("avr-none")
            .join("debug")
            .join("es.elf")
    }

    fn scale_mul_div(value: u64, numerator: u64, denominator: u64) -> u64 {
        (((value as u128) * (numerator as u128)) / (denominator as u128))
            .min(u64::MAX as u128) as u64
    }

    fn scale_div(value: u64, numerator: u64, denominator: u64) -> u64 {
        (((value as u128) * (numerator as u128)) / (denominator as u128))
            .min(u64::MAX as u128) as u64
    }
}

#[cfg(feature = "std")]
pub use runtime::{
    PublicCaseConfig as CaseConfig, PublicCaseMetadata as CaseMetadata, PublicSim as Sim,
};

pub mod prelude {
    pub use crate::{DigitalPin, PinTransition};

    #[cfg(feature = "std")]
    pub use crate::{CaseConfig, CaseMetadata, Sim};
}

#[cfg(all(test, feature = "std"))]
use runtime::run_case;

#[cfg(all(test, feature = "std"))]
include!(concat!(env!("OUT_DIR"), "/generated_sim_tests.rs"));
