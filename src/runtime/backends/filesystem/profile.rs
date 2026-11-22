
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("ASCOUT_FS_PROFILE").is_some())
}

pub(crate) struct Profile {
    operation: &'static str,
    enabled: bool,
    phases: Vec<(&'static str, usize, Duration)>,
}

impl Profile {
    pub(crate) fn new(operation: &'static str) -> Self {
        Self {
            operation,
            enabled: enabled(),
            phases: Vec::new(),
        }
    }

    pub(crate) fn run<T>(&mut self, phase: &'static str, action: impl FnOnce() -> T) -> T {
        if !self.enabled {
            return action();
        }
        let started = Instant::now();
        let value = action();
        self.add(phase, started.elapsed());
        value
    }

    pub(crate) fn add(&mut self, phase: &'static str, elapsed: Duration) {
        if !self.enabled {
            return;
        }
        if let Some(entry) = self.phases.iter_mut().find(|entry| entry.0 == phase) {
            entry.1 += 1;
            entry.2 += elapsed;
        } else {
            self.phases.push((phase, 1, elapsed));
        }
    }

    pub(crate) fn finish(self) {
        if !self.enabled {
            return;
        }
        let phases = self
            .phases
            .iter()
            .map(|(phase, count, total)| {
                format!("{phase}={:.3}ms({count})", total.as_secs_f64() * 1000.0)
            })
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("filesystem-profile: op={} {}", self.operation, phases);
    }
}
