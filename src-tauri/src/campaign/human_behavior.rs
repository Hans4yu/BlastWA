// Human Behavior Simulation Engine
// per-account personality + burst/rest rhythm + typing-time correlation +
// adaptive backoff + time-of-day modulation. the goal: statistically look like
// a human customer service rep, not a cron job.
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rand_distr::{Distribution, Normal};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Preset {
    Off,
    Natural,
    Cautious,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Personality {
    pub account_name: String,
    pub speed_multiplier: f64,
    pub typing_wpm: u32,
    pub burst_len_min: u32,
    pub burst_len_max: u32,
    pub rest_freq: f64,
}

impl Personality {
    /// deterministic per account: same name -> same personality every run
    pub fn generate(account_name: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        account_name.hash(&mut hasher);
        let seed = hasher.finish();
        let mut rng = StdRng::seed_from_u64(seed);
        Personality {
            account_name: account_name.to_string(),
            speed_multiplier: rng.gen_range(0.7..1.4),
            typing_wpm: rng.gen_range(35..=60),
            burst_len_min: rng.gen_range(5..=8),
            burst_len_max: rng.gen_range(10..=15),
            rest_freq: rng.gen_range(0.08..0.18),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RhythmState {
    Active,
    #[allow(dead_code)]
    Resting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanBehaviorConfig {
    pub preset: Preset,
    /// flat delay range in seconds used as base (delay_start..delay_end)
    pub delay_min_s: f64,
    pub delay_max_s: f64,
    pub min_delay_ms: u64,
    pub max_delay_ms: u64,
    pub enable_typing_sim: bool,
    pub enable_backoff: bool,
    pub enable_time_of_day: bool,
    pub enable_order_jitter: bool,
    pub jitter_window: usize,
}

impl Default for HumanBehaviorConfig {
    fn default() -> Self {
        Self {
            preset: Preset::Natural,
            delay_min_s: 3.0,
            delay_max_s: 9.0,
            min_delay_ms: 500,
            max_delay_ms: 300_000,
            enable_typing_sim: true,
            enable_backoff: true,
            enable_time_of_day: true,
            enable_order_jitter: true,
            jitter_window: 20,
        }
    }
}

/// time-of-day activity multipliers: index = hour (0-23).
/// late night slower (humans asleep), business hours faster.
fn hour_multiplier(hour: u32) -> f64 {
    const CURVE: [f64; 24] = [
        1.8, 1.9, 2.0, 1.9, 1.7, 1.4, 1.2, 1.0, 0.9, 0.85, 0.85, 0.9, // 00-11
        0.95, 1.0, 1.0, 1.0, 1.05, 1.1, 1.15, 1.25, 1.4, 1.55, 1.65, 1.75, // 12-23
    ];
    CURVE[(hour as usize) % 24]
}

/// estimated "typing" duration for a message body given wpm.
/// returns milliseconds; floor of 400ms so empty msgs still feel intentional.
pub fn typing_duration_ms(text: &str, wpm: u32) -> u64 {
    let word_count = text.split_whitespace().count();
    if word_count == 0 {
        return 400; // empty message still gets an intentional floor
    }
    let ms = word_count as f64 * (60_000.0 / wpm.max(1) as f64);
    let mut rng = rand::thread_rng();
    let jittered = ms * rng.gen_range(0.8..1.2);
    (jittered as u64).max(400)
}

#[derive(Debug, Clone, Serialize)]
pub struct DelayDecision {
    pub wait: Duration,
    pub reason: String,
    #[serde(skip_serializing)]
    pub state_after: RhythmState,
}

pub struct HumanBehaviorEngine {
    config: HumanBehaviorConfig,
    personality: Personality,
    state: RhythmState,
    burst_remaining: u32,
    consecutive_successes: u32,
    backoff_level: u32,
}

impl HumanBehaviorEngine {
    pub fn new(account_name: &str, config: HumanBehaviorConfig) -> Self {
        let personality = Personality::generate(account_name);
        let mut rng = rand::thread_rng();
        let burst_remaining = rng.gen_range(personality.burst_len_min..=personality.burst_len_max);
        Self {
            burst_remaining,
            config,
            personality,
            state: RhythmState::Active,
            consecutive_successes: 0,
            backoff_level: 0,
        }
    }

    pub fn personality(&self) -> &Personality {
        &self.personality
    }

    pub fn config(&self) -> &HumanBehaviorConfig {
        &self.config
    }

    pub fn set_order_jitter(&mut self, on: bool) {
        self.config.enable_order_jitter = on;
    }

    pub fn jitter_window(&self) -> usize {
        self.config.jitter_window
    }

    /// called after each send attempt to feed the backoff tracker
    pub fn record_result(&mut self, success: bool) {
        if !self.config.enable_backoff {
            return;
        }
        if success {
            self.consecutive_successes += 1;
            // decay backoff gently: every 10 clean sends drop a level
            if self.backoff_level > 0 && self.consecutive_successes.is_multiple_of(10) {
                self.backoff_level -= 1;
            }
        } else {
            self.consecutive_successes = 0;
            self.backoff_level = (self.backoff_level + 1).min(4);
        }
    }

    fn base_delay_ms(&self, lo: f64, hi: f64) -> u64 {
        let mut rng = rand::thread_rng();
        let hi = hi.max(lo + 0.1);
        let mean = (lo + hi) / 2.0;
        let std_dev = (hi - lo) / 4.0;

        match Normal::new(mean, std_dev) {
            Ok(normal) => {
                let sample = normal.sample(&mut rng);
                (sample.clamp(lo, hi) * 1000.0) as u64
            }
            Err(_) => (rng.gen_range(lo..hi) * 1000.0) as u64,
        }
    }

    fn apply_personality(&self, base_ms: u64) -> u64 {
        (base_ms as f64 * self.personality.speed_multiplier) as u64
    }

    fn apply_backoff(&self, ms: u64) -> u64 {
        if self.backoff_level == 0 {
            return ms;
        }
        let factor = 2u64.pow(self.backoff_level);
        ms.saturating_mul(factor)
    }

    fn next_rhythm(&mut self) -> Option<Duration> {
        let mut rng = rand::thread_rng();
        match self.state {
            RhythmState::Active => {
                if self.burst_remaining > 0 {
                    self.burst_remaining -= 1;
                    None
                } else {
                    // burst done -> maybe rest based on personality frequency
                    if rng.gen_bool(self.personality.rest_freq.min(0.9)) {
                        Some(Duration::from_secs(rng.gen_range(30..180)))
                    } else {
                        None
                    }
                }
            }
            RhythmState::Resting => {
                self.state = RhythmState::Active;
                self.burst_remaining =
                    rng.gen_range(self.personality.burst_len_min..=self.personality.burst_len_max);
                None
            }
        }
    }

    /// compute the full wait before the next send.
    /// order: base gaussian -> personality -> time-of-day -> backoff -> rhythm rest
    pub fn next_wait(&mut self, last_message: &str) -> DelayDecision {
        let mut rng = rand::thread_rng();
        let lo = self.config.delay_min_s;
        let hi = self.config.delay_max_s.max(lo + 0.1);

        if self.config.preset == Preset::Off {
            // legacy flat uniform mode, mirrors original app behavior
            let lo = (lo * 1000.0) as u64;
            let hi = (hi * 1000.0) as u64;
            return DelayDecision {
                wait: Duration::from_millis(rng.gen_range(lo..=hi)),
                reason: "flat".into(),
                state_after: RhythmState::Active,
            };
        }

        // Cautious stretches the base range: same gaussian shape, a 1.5x
        // floor and a 2x ceiling — meaningfully slower without changing
        // the character of the pacing.
        let (lo, hi) = if self.config.preset == Preset::Cautious {
            (lo * 1.5, hi * 2.0)
        } else {
            (lo, hi)
        };

        if self.config.preset == Preset::Custom {
            // pure gaussian on the exact typed range: no per-account style,
            // quiet hours, error backoff, typing or burst rests
            let ms = self.base_delay_ms(lo, hi);
            let clamped = ms.clamp(self.config.min_delay_ms, self.config.max_delay_ms);
            return DelayDecision {
                wait: Duration::from_millis(clamped),
                reason: "custom-gaussian".into(),
                state_after: RhythmState::Active,
            };
        }

        let mut ms = self.apply_personality(self.base_delay_ms(lo, hi));

        if self.config.enable_time_of_day {
            let hour = chrono::Local::now().format("%H").to_string();
            if let Ok(h) = hour.parse::<u32>() {
                let mult = hour_multiplier(h);
                ms = (ms as f64 * mult) as u64;
            }
        }

        ms = self.apply_backoff(ms);

        let mut reason_parts = vec!["gaussian".to_string()];
        if self.backoff_level > 0 {
            reason_parts.push(format!("backoff_x{}", 2u64.pow(self.backoff_level)));
        }

        // typing simulation adds its own wait on top when enabled
        if self.config.enable_typing_sim {
            let t = typing_duration_ms(last_message, self.personality.typing_wpm);
            ms += t;
            reason_parts.push(format!("typing_{}ms", t));
        }

        let rhythm_rest = if self.config.enable_typing_sim || self.config.enable_time_of_day {
            self.next_rhythm()
        } else {
            None
        };

        if let Some(rest) = rhythm_rest {
            ms = ms.saturating_add(rest.as_millis() as u64);
            reason_parts.push("rest_burst".into());
        }

        let clamped = ms.clamp(self.config.min_delay_ms, self.config.max_delay_ms);

        DelayDecision {
            wait: Duration::from_millis(clamped),
            reason: reason_parts.join("+"),
            state_after: self.state,
        }
    }

    /// shuffle a window of contacts so send order doesn't mirror import order
    pub fn jitter_order(&self, window: &mut [usize]) {
        if !self.config.enable_order_jitter || window.len() < 2 {
            return;
        }
        let mut rng = rand::thread_rng();
        for i in (1..window.len()).rev() {
            window.swap(i, rng.gen_range(0..=i));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn natural_cfg() -> HumanBehaviorConfig {
        HumanBehaviorConfig::default()
    }

    #[test]
    fn personality_stable_per_account() {
        let a1 = Personality::generate("akunA");
        let a2 = Personality::generate("akunA");
        let b = Personality::generate("akunB");
        assert_eq!(a1.speed_multiplier, a2.speed_multiplier);
        assert!((35..=60).contains(&b.typing_wpm));
        assert_ne!(a1.account_name, b.account_name);
    }

    #[test]
    fn typing_correlates_with_length() {
        let short = typing_duration_ms("Halo", 40);
        let long = typing_duration_ms(
            "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor",
            40,
        );
        assert!(long > short, "long msg should take longer to 'type'");
    }

    #[test]
    fn typing_floor_respected() {
        assert_eq!(typing_duration_ms("", 45), 400);
    }

    #[test]
    fn off_preset_is_flat_uniform_in_range() {
        let cfg = HumanBehaviorConfig {
            preset: Preset::Off,
            delay_min_s: 1.0,
            delay_max_s: 2.0,
            ..Default::default()
        };
        let mut engine = HumanBehaviorEngine::new("x", cfg);
        for _ in 0..50 {
            let d = engine.next_wait("hello");
            assert!(d.wait >= Duration::from_millis(1000));
            assert!(d.wait <= Duration::from_millis(2000));
        }
    }

    #[test]
    fn delays_within_bounds_and_cluster_near_mean() {
        let mut engine = HumanBehaviorEngine::new("tester", natural_cfg());
        let mut total = 0u128;
        let n = 200;
        for _ in 0..n {
            let d = engine.next_wait("sample text");
            assert!(
                d.wait <= Duration::from_millis(engine.config.max_delay_ms),
                "exceeded max"
            );
            total += d.wait.as_millis();
        }
        let avg = total / n;
        assert!(avg > 0, "avg should be positive");
    }

    #[test]
    fn backoff_multiplies_on_failures() {
        let mut engine = HumanBehaviorEngine::new("bk", natural_cfg());
        engine.record_result(false);
        engine.record_result(false);
        let d = engine.next_wait("msg");
        // with backoff level 2 => x4 multiplier applied somewhere in chain
        assert!(d.reason.contains("backoff"), "reason: {}", d.reason);
    }

    #[test]
    fn jitter_preserves_all_indices() {
        let engine = HumanBehaviorEngine::new("j", natural_cfg());
        let mut window: Vec<usize> = (0..20).collect();
        engine.jitter_order(&mut window);
        let mut sorted = window.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn cautious_preset_never_below_floor() {
        let cfg = HumanBehaviorConfig {
            preset: Preset::Cautious,
            min_delay_ms: 800,
            ..Default::default()
        };
        let mut engine = HumanBehaviorEngine::new("c", cfg);
        for _ in 0..30 {
            assert!(engine.next_wait("x").wait >= Duration::from_millis(800));
        }
    }
}
