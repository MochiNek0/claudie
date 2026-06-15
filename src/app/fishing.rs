use std::time::{Duration, Instant};

const BITE_WAIT_MIN_MS: u64 = 1_600;
const BITE_WAIT_SPREAD_MS: u64 = 3_200;
const RESULT_VISIBLE: Duration = Duration::from_secs(3);
const REEL_LIMIT: Duration = Duration::from_secs(32);
const MISS_GRACE: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FishingPhase {
    Inactive,
    Waiting,
    Reeling,
    Caught,
    Missed,
}

#[derive(Clone, Debug)]
pub(crate) struct FishingState {
    pub(crate) phase: FishingPhase,
    pub(crate) phase_started_at: Instant,
    pub(crate) bite_at: Option<Instant>,
    pub(crate) tension: f32,
    pub(crate) progress: f32,
    pub(crate) target_center: f32,
    pub(crate) target_half_width: f32,
    next_target_shift_at: Instant,
    rng: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FishingTick {
    None,
    Bite,
    Caught,
    Missed,
    Finished,
}

impl Default for FishingState {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            phase: FishingPhase::Inactive,
            phase_started_at: now,
            bite_at: None,
            tension: 0.5,
            progress: 0.0,
            target_center: 0.5,
            target_half_width: 0.16,
            next_target_shift_at: now,
            rng: seed_from(now),
        }
    }
}

impl FishingState {
    pub(crate) fn start(&mut self) {
        let now = Instant::now();
        self.rng ^= seed_from(now).rotate_left(17);
        self.phase = FishingPhase::Waiting;
        self.phase_started_at = now;
        self.bite_at = Some(now + self.bite_wait());
        self.tension = 0.5;
        self.progress = 0.0;
        self.target_center = 0.5;
        self.target_half_width = 0.16;
        self.next_target_shift_at = now;
    }

    pub(crate) fn stop(&mut self) {
        let now = Instant::now();
        self.phase = FishingPhase::Inactive;
        self.phase_started_at = now;
        self.bite_at = None;
        self.progress = 0.0;
        self.tension = 0.5;
    }

    pub(crate) fn is_active(&self) -> bool {
        !matches!(self.phase, FishingPhase::Inactive)
    }

    pub(crate) fn is_reeling(&self) -> bool {
        matches!(self.phase, FishingPhase::Reeling)
    }

    pub(crate) fn input(&mut self) -> bool {
        if !self.is_reeling() {
            return self.is_active();
        }
        self.tension = (self.tension + 0.07).clamp(0.0, 1.0);
        true
    }

    pub(crate) fn tick(&mut self) -> FishingTick {
        let now = Instant::now();
        match self.phase {
            FishingPhase::Inactive => FishingTick::None,
            FishingPhase::Waiting => {
                if self.bite_at.is_some_and(|bite_at| now >= bite_at) {
                    self.start_reeling(now);
                    FishingTick::Bite
                } else {
                    FishingTick::None
                }
            }
            FishingPhase::Reeling => self.tick_reeling(now),
            FishingPhase::Caught | FishingPhase::Missed => {
                if now.duration_since(self.phase_started_at) >= RESULT_VISIBLE {
                    self.stop();
                    FishingTick::Finished
                } else {
                    FishingTick::None
                }
            }
        }
    }

    pub(crate) fn target_range(&self) -> (f32, f32) {
        (
            (self.target_center - self.target_half_width).clamp(0.0, 1.0),
            (self.target_center + self.target_half_width).clamp(0.0, 1.0),
        )
    }

    fn start_reeling(&mut self, now: Instant) {
        self.phase = FishingPhase::Reeling;
        self.phase_started_at = now;
        self.tension = 0.44 + self.next_unit() * 0.12;
        self.progress = 0.2;
        self.shift_target(now);
    }

    fn tick_reeling(&mut self, now: Instant) -> FishingTick {
        if now >= self.next_target_shift_at {
            self.shift_target(now);
        }

        let drift = 0.006 + self.next_unit() * 0.004;
        self.tension = (self.tension - drift).clamp(0.0, 1.0);
        let (target_min, target_max) = self.target_range();
        if self.tension >= target_min && self.tension <= target_max {
            self.progress = (self.progress + 0.010).clamp(0.0, 1.0);
        } else {
            self.progress = (self.progress - 0.008).clamp(0.0, 1.0);
        }

        if self.progress >= 1.0 {
            self.phase = FishingPhase::Caught;
            self.phase_started_at = now;
            FishingTick::Caught
        } else if now.duration_since(self.phase_started_at) >= REEL_LIMIT
            || (self.progress <= 0.0 && now.duration_since(self.phase_started_at) >= MISS_GRACE)
        {
            self.phase = FishingPhase::Missed;
            self.phase_started_at = now;
            FishingTick::Missed
        } else {
            FishingTick::None
        }
    }

    fn shift_target(&mut self, now: Instant) {
        self.target_center = 0.26 + self.next_unit() * 0.48;
        self.target_half_width = 0.09 + self.next_unit() * 0.045;
        let wait_ms = 800 + (self.next_unit() * 700.0) as u64;
        self.next_target_shift_at = now + Duration::from_millis(wait_ms);
    }

    fn bite_wait(&mut self) -> Duration {
        Duration::from_millis(BITE_WAIT_MIN_MS + (self.next_u64() % BITE_WAIT_SPREAD_MS))
    }

    fn next_unit(&mut self) -> f32 {
        (self.next_u64() as f64 / u64::MAX as f64) as f32
    }

    fn next_u64(&mut self) -> u64 {
        self.rng = self
            .rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.rng
    }
}

fn seed_from(now: Instant) -> u64 {
    let nanos = now.elapsed().as_nanos() as u64;
    nanos ^ (&now as *const Instant as usize as u64).rotate_left(23)
}
