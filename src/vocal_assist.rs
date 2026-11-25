//! GP-AUTO main-vocal assist: keep the soundchecked level, reduce only when it peaks.
//!
//! The rule (user, 2026-11-21): measure the main vocal while the singer soundchecks
//! and keep that level. The only automatic change is a bounded level reduction when
//! the vocal peaks above the measured level. No spill classification or treatment.
//!
//! The sample loop has fixed storage and no allocation, locks, I/O, log or pow;
//! coefficients and thresholds are prepared outside rendering.
use crate::show::Counter;
use serde::{Deserialize, Serialize};

/// Fixed detector time constant. A one-pole mean square with 10 ms time constant
/// tracks the same short-term level as the 20 ms GP-METER RMS windows the
/// soundcheck reference is measured from.
pub const DETECTOR_MS: f64 = 10.;
/// Hard bound on the automatic cut. Larger cuts are operator decisions.
pub const MAX_CUT_MDB: u32 = 12000;

/// Where the reference level came from. Manual entry is never relabelled as measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    Manual,
    Soundcheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub bypass: bool,
    /// Measured (or manually entered) short-term RMS level the singer is kept at.
    pub reference_mdb: i32,
    /// Allowed rise above the reference before any reduction.
    pub headroom_mdb: u32,
    /// Largest automatic reduction, 0.1..12 dB.
    pub max_cut_mdb: u32,
    pub attack_us: u32,
    /// Automatic recovery time once the vocal falls back below the threshold.
    pub release_ms: u32,
    pub origin: Origin,
    /// Active soundcheck windows behind the reference; zero exactly when manual.
    pub evidence_windows: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            bypass: true,
            reference_mdb: -18000,
            headroom_mdb: 3000,
            max_cut_mdb: 6000,
            attack_us: 5000,
            release_ms: 400,
            origin: Origin::Manual,
            evidence_windows: 0,
        }
    }
}
impl Config {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn validate(self) -> Result<(), String> {
        let stepped = |v: i64, lo: i64, hi: i64, step: i64| (lo..=hi).contains(&v) && v % step == 0;
        if !stepped(self.reference_mdb.into(), -60000, 0, 100)
            || !stepped(self.headroom_mdb.into(), 0, 12000, 100)
            || !stepped(self.max_cut_mdb.into(), 100, MAX_CUT_MDB.into(), 100)
            || !stepped(self.attack_us.into(), 100, 200000, 100)
            || !(10..=5000).contains(&self.release_ms)
            || i64::from(self.reference_mdb) + i64::from(self.headroom_mdb) > 0
            || (self.origin == Origin::Manual) != (self.evidence_windows == 0)
            || self.evidence_windows > MAX_WINDOWS as u32
        {
            return Err("vocal assist range".into());
        }
        Ok(())
    }
    pub fn threshold_mdb(self) -> i32 {
        self.reference_mdb + self.headroom_mdb as i32
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Prepared {
    pub config: Config,
    threshold_ms: f64,
    min_gain: f64,
    detector: f64,
    attack: f64,
    release: f64,
}
impl Prepared {
    pub fn new(config: Config, rate: u32) -> Result<Self, String> {
        config.validate()?;
        if !(8000..=192000).contains(&rate) {
            return Err("vocal assist sample rate".into());
        }
        let rate = rate as f64;
        Ok(Self {
            config,
            threshold_ms: 10f64.powf(config.threshold_mdb() as f64 / 10000.),
            min_gain: 10f64.powf(-(config.max_cut_mdb as f64) / 20000.),
            detector: 1. - (-1. / (DETECTOR_MS * 0.001 * rate)).exp(),
            attack: (-1. / (config.attack_us as f64 * 0.000001 * rate)).exp(),
            release: (-1. / (config.release_ms as f64 * 0.001 * rate)).exp(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Disabled and fully recovered: the input passes unchanged.
    Bypassed,
    /// Enabled, vocal at or below the threshold, no reduction.
    Idle,
    /// Vocal above the threshold; reduction is applied or deepening.
    Reducing,
    /// Back below the threshold (or disabled); gain returning to unity.
    Recovering,
}

#[derive(Debug, Clone, Copy)]
pub struct Assist {
    prepared: Prepared,
    pub state: State,
    mean_square: f64,
    gain: f64,
    /// Lowest gain since the current settings were applied.
    deepest: f64,
    /// Reduction events (idle → reducing) since the current settings were applied.
    onsets: u64,
}
impl Assist {
    pub fn new(prepared: Prepared) -> Self {
        Self {
            prepared,
            state: State::Bypassed,
            mean_square: 0.,
            gain: 1.,
            deepest: 1.,
            onsets: 0,
        }
    }
    /// Retain detector and gain across edits, so an operator edit mid-reduction
    /// neither jumps nor restarts; the new settings govern from the next sample.
    /// Source epoch reset uses new().
    pub fn apply(&mut self, prepared: Prepared) {
        if self.prepared.config == prepared.config {
            return;
        }
        self.prepared = prepared;
        self.deepest = self.gain;
        self.onsets = 0;
    }
    pub fn linear_gain(&self) -> f64 {
        self.gain
    }
    /// Causal offline/block adapter; identical state and sample path to the live strip.
    pub fn process_block(&mut self, samples: &mut [f64]) {
        for sample in samples {
            *sample = self.tick(*sample);
        }
    }
    pub fn tick(&mut self, input: f64) -> f64 {
        if !input.is_finite() {
            self.mean_square = 0.;
            return 0.;
        }
        let p = self.prepared;
        if p.config.bypass && self.gain == 1. {
            self.state = State::Bypassed;
            self.mean_square = 0.;
            return input;
        }
        self.mean_square += p.detector * (input * input - self.mean_square);
        if self.mean_square < 1e-30 {
            self.mean_square = 0.;
        }
        let above = !p.config.bypass && self.mean_square > p.threshold_ms;
        let desired = if above {
            // Exactly the gain that holds the detector at the threshold, bounded.
            (p.threshold_ms / self.mean_square).sqrt().max(p.min_gain)
        } else {
            1.
        };
        let coefficient = if desired < self.gain {
            p.attack
        } else {
            p.release
        };
        self.gain = desired + coefficient * (self.gain - desired);
        if self.gain > 1. - 1e-5 && desired == 1. {
            self.gain = 1.;
        }
        if self.gain < self.deepest {
            self.deepest = self.gain;
        }
        let next = if above {
            State::Reducing
        } else if self.gain < 1. {
            State::Recovering
        } else if p.config.bypass {
            State::Bypassed
        } else {
            State::Idle
        };
        // One reduction event lasts until full recovery: detector ripple at the
        // threshold or a re-peak while recovering does not count again.
        if next == State::Reducing && matches!(self.state, State::Idle | State::Bypassed) {
            self.onsets = self.onsets.saturating_add(1);
        }
        self.state = next;
        input * self.gain
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub state: State,
    /// Current positive reduction.
    pub reduction_mdb: u32,
    /// Deepest reduction since these settings were applied.
    pub deepest_mdb: u32,
    /// Reduction events since these settings were applied; an event ends at full recovery.
    pub onsets: Counter,
}
impl Observation {
    pub fn validate(&self) -> Result<(), String> {
        if self.reduction_mdb > MAX_CUT_MDB
            || self.deepest_mdb > MAX_CUT_MDB
            || self.reduction_mdb > self.deepest_mdb
            || (self.state == State::Bypassed && self.reduction_mdb != 0)
            || (self.state == State::Idle && self.reduction_mdb != 0)
        {
            return Err("vocal assist observation".into());
        }
        Ok(())
    }
}
/// Numeric source capture; logarithmic presentation stays on the observer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceObservation {
    state: State,
    gain: f64,
    deepest: f64,
    onsets: u64,
}
fn attenuation(gain: f64) -> u32 {
    (-20000. * gain.max(1e-12).log10())
        .round()
        .clamp(0., MAX_CUT_MDB as f64) as u32
}
impl SourceObservation {
    pub fn readback(self) -> Observation {
        let reduction_mdb = if matches!(self.state, State::Bypassed | State::Idle) {
            0
        } else {
            attenuation(self.gain)
        };
        Observation {
            state: self.state,
            reduction_mdb,
            deepest_mdb: attenuation(self.deepest).max(reduction_mdb),
            onsets: Counter(self.onsets),
        }
    }
}
impl Assist {
    pub(crate) fn source_observation(&self) -> SourceObservation {
        SourceObservation {
            state: self.state,
            gain: self.gain,
            deepest: self.deepest,
            onsets: self.onsets,
        }
    }
    /// Control/readback path only: logarithm never runs in tick().
    pub fn observation(&self) -> Observation {
        self.source_observation().readback()
    }
}

// ---------------------------------------------------------------------------
// Soundcheck observation window → derived reference with provenance.

/// Most windows in one soundcheck (two minutes at the 25 Hz Desk observer rate).
pub const MAX_WINDOWS: usize = 3000;
/// Least active singing: six seconds at 25 Hz, three at the full 50 Hz window rate.
pub const MIN_ACTIVE_WINDOWS: usize = 150;
/// Windows below this RMS do not count as singing.
pub const ACTIVE_FLOOR_MDB: i32 = -60000;
/// A reference this low means the mic is not gain-staged for a vocal.
pub const QUIET_REFERENCE_MDB: i32 = -50000;
/// GP-METER freshness bound.
pub const STALE_AGE_MS: u32 = 250;
/// Reference percentile over active windows: the loud singing the singer showed.
pub const REFERENCE_PERCENTILE: f64 = 0.9;

/// One GP-METER window of the vocal's pre-fader processed tap, as received.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub sequence: Counter,
    pub rms_mdbfs: Option<i32>,
    pub sample_peak_mdbfs: Option<i32>,
    pub silent: bool,
    pub clip_count: u32,
    /// Conservative acquisition age when Desk received it.
    pub age_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Prepared,
    /// Fewer than MIN_ACTIVE_WINDOWS of singing.
    TooShort,
    /// More than a fifth of windows were stale or unavailable.
    UnreliableMeter,
    /// Clipping during soundcheck: fix the input gain first.
    Clipped,
    /// The loud singing level is below QUIET_REFERENCE_MDB.
    TooQuiet,
    /// Malformed evidence: empty, oversize, out-of-order or out-of-range windows.
    InvalidEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurements {
    pub windows: u32,
    pub stale: u32,
    pub unavailable: u32,
    pub silent: u32,
    pub active: u32,
    pub first_sequence: Counter,
    pub last_sequence: Counter,
    pub rms_p10_mdbfs: i32,
    pub rms_p50_mdbfs: i32,
    pub rms_p90_mdbfs: i32,
    pub rms_p99_mdbfs: i32,
    pub peak_max_mdbfs: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    /// Enabled on success; the bypassed default on every refusal.
    pub config: Config,
    pub reason: Reason,
    pub measurements: Option<Measurements>,
}

fn percentile(sorted: &[i32], q: f64) -> i32 {
    let index = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

/// Derive the kept level from one soundcheck. Pure and deterministic; no audio I/O.
pub fn prepare_soundcheck(windows: &[Window]) -> Proposal {
    let refuse = |reason, measurements| Proposal {
        config: Config::default(),
        reason,
        measurements,
    };
    if windows.is_empty() || windows.len() > MAX_WINDOWS {
        return refuse(Reason::InvalidEvidence, None);
    }
    if windows
        .windows(2)
        .any(|w| w[0].sequence.0 >= w[1].sequence.0)
        || windows.iter().any(|w| {
            w.rms_mdbfs.is_some_and(|v| !(-120000..=0).contains(&v))
                || w.sample_peak_mdbfs
                    .is_some_and(|v| !(-120000..=0).contains(&v))
                || w.rms_mdbfs.is_some() != w.sample_peak_mdbfs.is_some()
                || w.rms_mdbfs
                    .zip(w.sample_peak_mdbfs)
                    .is_some_and(|(r, p)| r > p)
        })
    {
        return refuse(Reason::InvalidEvidence, None);
    }
    let stale = windows.iter().filter(|w| w.age_ms > STALE_AGE_MS).count();
    let usable: Vec<_> = windows
        .iter()
        .filter(|w| w.age_ms <= STALE_AGE_MS && w.rms_mdbfs.is_some())
        .collect();
    let unavailable = windows.len() - stale - usable.len();
    let silent = usable.iter().filter(|w| w.silent).count();
    let mut rms: Vec<i32> = usable
        .iter()
        .filter(|w| !w.silent)
        .filter_map(|w| w.rms_mdbfs)
        .filter(|&v| v >= ACTIVE_FLOOR_MDB)
        .collect();
    rms.sort_unstable();
    let measurements = (!rms.is_empty()).then(|| Measurements {
        windows: windows.len() as u32,
        stale: stale as u32,
        unavailable: unavailable as u32,
        silent: silent as u32,
        active: rms.len() as u32,
        first_sequence: windows[0].sequence,
        last_sequence: windows[windows.len() - 1].sequence,
        rms_p10_mdbfs: percentile(&rms, 0.1),
        rms_p50_mdbfs: percentile(&rms, 0.5),
        rms_p90_mdbfs: percentile(&rms, REFERENCE_PERCENTILE),
        rms_p99_mdbfs: percentile(&rms, 0.99),
        peak_max_mdbfs: usable
            .iter()
            .filter_map(|w| w.sample_peak_mdbfs)
            .max()
            .unwrap_or(-120000),
    });
    if (stale + unavailable) * 5 > windows.len() {
        return refuse(Reason::UnreliableMeter, measurements);
    }
    if usable.iter().any(|w| w.clip_count > 0) {
        return refuse(Reason::Clipped, measurements);
    }
    let Some(m) = measurements.filter(|_| rms.len() >= MIN_ACTIVE_WINDOWS) else {
        return refuse(Reason::TooShort, measurements);
    };
    // Round to the 0.1 dB wire step, never above the measured level.
    let reference = m.rms_p90_mdbfs.div_euclid(100) * 100;
    if reference < QUIET_REFERENCE_MDB {
        return refuse(Reason::TooQuiet, Some(m));
    }
    let defaults = Config::default();
    let config = Config {
        bypass: false,
        reference_mdb: reference,
        headroom_mdb: defaults.headroom_mdb.min(reference.unsigned_abs()),
        origin: Origin::Soundcheck,
        evidence_windows: m.active,
        ..defaults
    };
    debug_assert!(config.validate().is_ok());
    Proposal {
        config,
        reason: Reason::Prepared,
        measurements: Some(m),
    }
}

// ---------------------------------------------------------------------------
// Desk control process: one bounded request on stdin, one document on stdout.

pub const REQUEST_VERSION: u32 = 1;
pub const REQUEST_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub show_id: String,
    pub epoch: Counter,
    pub input: String,
    /// Always `input-NN:processed`, the actual pre-fader strip tap for `input`.
    pub tap: String,
    pub windows: Vec<Window>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    pub show_id: String,
    pub epoch: Counter,
    pub input: String,
    pub tap: String,
    pub rule: String,
    pub proposal: Proposal,
}
pub const RULE: &str = "gp-auto-main-vocal-2026-11-21";

impl Request {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > REQUEST_BYTES {
            return Err("vocal soundcheck request capacity".into());
        }
        let request: Self = crate::show::decode_bounded(bytes, REQUEST_BYTES)?;
        if request.version != REQUEST_VERSION {
            return Err("vocal soundcheck request version".into());
        }
        crate::processing_wire::input_index(&request.input)?;
        if request.tap != format!("{}:processed", request.input) {
            return Err("vocal soundcheck tap must be the input's processed tap".into());
        }
        if request.windows.len() > MAX_WINDOWS {
            return Err("vocal soundcheck window capacity".into());
        }
        Ok(request)
    }
    pub fn prepare(&self) -> Document {
        Document {
            version: REQUEST_VERSION,
            show_id: self.show_id.clone(),
            epoch: self.epoch,
            input: self.input.clone(),
            tap: self.tap.clone(),
            rule: RULE.into(),
            proposal: prepare_soundcheck(&self.windows),
        }
    }
}

pub fn prepare_stream() -> Result<(), String> {
    use std::io::{Read, Write};
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let document = Request::decode(&bytes)?.prepare();
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &serde_json::json!({ "document": document }))
        .map_err(|e| e.to_string())?;
    writeln!(out)
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
}

pub fn prepare_file(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> crate::inventory::Result<()> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(source)?
        .take(REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let document = Request::decode(&bytes)?.prepare();
    crate::automix::write_json(destination, &document)
}

#[cfg(test)]
mod tests {
    use super::*;
    const RATE: u32 = 48000;
    fn enabled(reference_mdb: i32) -> Config {
        Config {
            bypass: false,
            reference_mdb,
            ..Config::default()
        }
    }
    fn sine(amplitude: f64, n: usize, offset: usize) -> impl Iterator<Item = f64> {
        (offset..offset + n)
            .map(move |i| amplitude * (std::f64::consts::TAU * 220. * i as f64 / 48000.).sin())
    }
    fn db(x: f64) -> f64 {
        20. * x.log10()
    }

    #[test]
    fn bypass_is_exact_identity() {
        let mut a = Assist::new(Prepared::new(Config::default(), RATE).unwrap());
        for x in sine(0.9, 4800, 0) {
            assert_eq!(a.tick(x), x);
        }
        assert_eq!(a.state, State::Bypassed);
        assert_eq!(a.observation().reduction_mdb, 0);
    }

    #[test]
    fn keeps_soundcheck_level_and_reduces_only_peaks() {
        // Reference −20 dBFS RMS, +3 dB headroom → threshold −17 dBFS RMS.
        let mut a = Assist::new(Prepared::new(enabled(-20000), RATE).unwrap());
        // A sine at −20 dBFS RMS (peak 0.1414) is the soundcheck level: untouched.
        let at_level = 0.1 * 2f64.sqrt();
        for x in sine(at_level, 48000, 0) {
            a.tick(x);
        }
        assert_eq!(a.linear_gain(), 1.);
        assert_eq!(a.state, State::Idle);
        // +9 dB peak: held near the threshold, i.e. about a 6 dB cut (= max cut).
        for x in sine(at_level * 10f64.powf(9. / 20.), 24000, 48000) {
            a.tick(x);
        }
        assert_eq!(a.state, State::Reducing);
        let cut = -db(a.linear_gain());
        assert!((5.5..=6.0 + 1e-9).contains(&cut), "cut {cut}");
        // +4 dB: about a 1 dB cut, bounded and proportional.
        let mut b = Assist::new(Prepared::new(enabled(-20000), RATE).unwrap());
        for x in sine(at_level * 10f64.powf(4. / 20.), 48000, 0) {
            b.tick(x);
        }
        let cut = -db(b.linear_gain());
        assert!((0.7..=1.3).contains(&cut), "cut {cut}");
        assert_eq!(b.observation().onsets, Counter(1));
    }

    #[test]
    fn cut_never_exceeds_bound_and_recovers_automatically() {
        let config = Config {
            max_cut_mdb: 4000,
            release_ms: 100,
            ..enabled(-30000)
        };
        let mut a = Assist::new(Prepared::new(config, RATE).unwrap());
        for x in sine(1.0, 24000, 0) {
            a.tick(x);
            assert!(a.linear_gain() >= 10f64.powf(-4. / 20.) - 1e-12);
        }
        let o = a.observation();
        assert_eq!(o.state, State::Reducing);
        assert_eq!(o.reduction_mdb, 4000);
        assert_eq!(o.deepest_mdb, 4000);
        // Silence: recovers to exact unity within a bounded time, no operator action.
        let mut recovering = false;
        for _ in 0..96000 {
            a.tick(0.);
            recovering |= a.state == State::Recovering;
        }
        assert!(recovering);
        assert_eq!(a.linear_gain(), 1.);
        assert_eq!(a.state, State::Idle);
        let o = a.observation();
        assert_eq!((o.reduction_mdb, o.deepest_mdb), (0, 4000));
    }

    #[test]
    fn edit_mid_reduction_keeps_gain_continuous() {
        let mut a = Assist::new(Prepared::new(enabled(-30000), RATE).unwrap());
        for x in sine(0.5, 9600, 0) {
            a.tick(x);
        }
        let before = a.linear_gain();
        assert!(before < 0.9);
        // Operator raises the reference mid-reduction: no step, release toward unity.
        a.apply(Prepared::new(enabled(-10000), RATE).unwrap());
        assert_eq!(a.linear_gain(), before);
        let mut previous = before;
        for x in sine(0.5, 4800, 9600) {
            a.tick(x);
            assert!(a.linear_gain() >= previous - 1e-12);
            assert!(a.linear_gain() - previous < 0.01);
            previous = a.linear_gain();
        }
        // Disabling mid-reduction also releases instead of stepping.
        let mut b = Assist::new(Prepared::new(enabled(-30000), RATE).unwrap());
        for x in sine(0.5, 9600, 0) {
            b.tick(x);
        }
        b.apply(Prepared::new(Config::default(), RATE).unwrap());
        let g = b.linear_gain();
        b.tick(0.5);
        assert!(b.linear_gain() >= g && b.linear_gain() - g < 0.01);
        assert_eq!(b.state, State::Recovering);
        for _ in 0..240000 {
            b.tick(0.5);
        }
        assert_eq!(b.state, State::Bypassed);
        assert_eq!(b.tick(0.5), 0.5);
    }

    #[test]
    fn nonfinite_input_is_contained() {
        let mut a = Assist::new(Prepared::new(enabled(-30000), RATE).unwrap());
        assert_eq!(a.tick(f64::NAN), 0.);
        assert_eq!(a.tick(f64::INFINITY), 0.);
        assert!(a.tick(0.01).is_finite());
    }

    #[test]
    fn config_bounds_and_provenance() {
        assert!(Config::default().validate().is_ok());
        for bad in [
            Config {
                max_cut_mdb: 12100,
                ..Config::default()
            },
            Config {
                max_cut_mdb: 0,
                ..Config::default()
            },
            Config {
                reference_mdb: -1000,
                headroom_mdb: 3000,
                ..Config::default()
            },
            Config {
                reference_mdb: -18050,
                ..Config::default()
            },
            Config {
                origin: Origin::Soundcheck,
                ..Config::default()
            },
            Config {
                evidence_windows: 200,
                ..Config::default()
            },
            Config {
                release_ms: 9,
                ..Config::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    fn window(sequence: u64, rms: i32) -> Window {
        Window {
            sequence: Counter(sequence),
            rms_mdbfs: Some(rms),
            sample_peak_mdbfs: Some((rms + 9000).min(0)),
            silent: false,
            clip_count: 0,
            age_ms: 40,
        }
    }
    fn singing(n: usize) -> Vec<Window> {
        (0..n)
            .map(|i| window(i as u64 * 2 + 1, -30000 + (i % 100) as i32 * 100))
            .collect()
    }

    #[test]
    fn soundcheck_derives_reference_from_loud_singing() {
        let mut w = singing(300);
        // Silence between phrases is ignored, not averaged in.
        for x in w.iter_mut().step_by(10) {
            *x = Window {
                rms_mdbfs: Some(-120000),
                sample_peak_mdbfs: Some(-120000),
                silent: true,
                ..window(x.sequence.0, 0)
            };
        }
        let p = prepare_soundcheck(&w);
        assert_eq!(p.reason, Reason::Prepared);
        let m = p.measurements.unwrap();
        assert_eq!((m.windows, m.silent, m.active), (300, 30, 270));
        assert_eq!(
            p.config.reference_mdb,
            m.rms_p90_mdbfs.div_euclid(100) * 100
        );
        assert!(p.config.reference_mdb > m.rms_p50_mdbfs);
        assert!(!p.config.bypass);
        assert_eq!(p.config.origin, Origin::Soundcheck);
        assert_eq!(p.config.evidence_windows, 270);
        p.config.validate().unwrap();
    }

    #[test]
    fn soundcheck_refusals_are_bypassed() {
        let check = |w: &[Window], reason| {
            let p = prepare_soundcheck(w);
            assert_eq!(p.reason, reason);
            assert_eq!(p.config, Config::default());
        };
        check(&singing(100), Reason::TooShort);
        check(&[], Reason::InvalidEvidence);
        let mut w = singing(300);
        w.swap(3, 4);
        check(&w, Reason::InvalidEvidence);
        let mut w = singing(300);
        for x in w.iter_mut().take(70) {
            x.age_ms = 400;
        }
        check(&w, Reason::UnreliableMeter);
        let mut w = singing(300);
        for x in w.iter_mut().skip(200).take(70) {
            x.rms_mdbfs = None;
            x.sample_peak_mdbfs = None;
        }
        check(&w, Reason::UnreliableMeter);
        let mut w = singing(300);
        w[150].clip_count = 1;
        check(&w, Reason::Clipped);
        let quiet: Vec<_> = (0..300).map(|i| window(i + 1, -55000)).collect();
        check(&quiet, Reason::TooQuiet);
        // A few stale windows are excluded, not fatal.
        let mut w = singing(300);
        for x in w.iter_mut().take(10) {
            x.age_ms = 300;
        }
        let p = prepare_soundcheck(&w);
        assert_eq!(p.reason, Reason::Prepared);
        assert_eq!(p.measurements.unwrap().stale, 10);
    }

    #[test]
    fn request_is_bound_to_the_processed_tap() {
        let request = |tap: &str| {
            serde_json::to_vec(&serde_json::json!({"version":1,
                "show_id":"6f0d4b3e-9f3a-4b8e-8a65-3c0e6b1d2a10","epoch":"1",
                "input":"input-03","tap":tap,"windows":singing(200)}))
            .unwrap()
        };
        let document = Request::decode(&request("input-03:processed"))
            .unwrap()
            .prepare();
        assert_eq!(document.rule, RULE);
        assert_eq!(document.proposal.reason, Reason::Prepared);
        assert!(Request::decode(&request("input-03:raw")).is_err());
        assert!(Request::decode(&request("input-04:processed")).is_err());
    }
}
