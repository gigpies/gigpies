//! GP-PA-MEASURE capture/excitation contract (`GP-PA-MEASURE-CAPTURE:1`).
//!
//! One bounded session plays a known logarithmic sweep into exactly one admitted PA
//! output path and captures one reserved measurement-mic slot on the same Stagebox
//! source-frame timeline. The reference is the generated excitation itself, aligned
//! by source frame, so SHR PA receives common-origin evidence without a loopback.
//!
//! This module is pure: plan decoding, admission, the excitation generator, the
//! session state machine and the capture records. It opens no device, changes no
//! route and allocates nothing per frame once a session exists. Host acquisition
//! (step 2) drives it; physical qualification is separate.
use crate::{
    measurement_wire::identifier,
    show::{Counter, Result},
    topology::EngineTopology,
};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub const CONTRACT: &str = "GP-PA-MEASURE-CAPTURE";
pub const VERSION: u32 = 1;
/// Excitation and mic share the Stagebox source-frame clock; nothing else qualifies.
pub const CLOCK_DOMAIN: &str = "stagebox-source";
pub const REFERENCE_TAP: &str = "excitation";
pub const RATE: u32 = 48000;
/// The owner's linear sweep end fades; also the ramp used to stop early.
pub const FADE_SAMPLES: usize = 256;
/// Peak level bounds of the known excitation (the bench preparation range).
pub const LEVEL_MIN_MDBFS: i32 = -60000;
pub const LEVEL_MAX_MDBFS: i32 = -30000;
pub const LEVEL_DEFAULT_MDBFS: i32 = -42000;
/// Owner `sweep` takes one record; `sweep_repeated` exactly four.
pub const REPEATS: [u8; 2] = [1, 4];
pub const TOTAL_SAMPLES: [usize; 2] = [32768, 65536];
pub const MIN_TAIL_SAMPLES: usize = 4096;
pub const MAX_SESSION_SAMPLES: usize = 4 * 65536;
/// Control-time allowance beyond the excitation itself (arm, ramp, restore).
pub const DEADLINE_MARGIN_MS: u64 = 2000;
/// Mic samples at or above this magnitude are clipped (owner refusal threshold).
pub const MIC_CLIP: f32 = 0.999;
pub const REQUEST_BYTES: usize = 8192;

/// Exact identity the plan was prepared against. Any difference refuses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Basis {
    pub show_id: String,
    pub source_epoch: Counter,
    pub map_revision: Counter,
    pub graph_generation: Counter,
    pub configuration_revision: Counter,
}

/// The speaker path: one output assigned in the admitted PA owner graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Path {
    pub output_index: usize,
}

/// Microphone correction is identified, never interpreted here; SHR PA applies it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub provenance: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mic {
    pub capture_slot: usize,
    pub calibration: Option<Calibration>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Excitation {
    pub kind: String,
    pub start_hz: u32,
    pub end_hz: u32,
    pub sweep_samples: usize,
    pub total_samples: usize,
    pub level_mdbfs: i32,
    pub repeats: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub contract: String,
    pub version: u32,
    pub id: String,
    pub position_id: String,
    pub basis: Basis,
    pub path: Path,
    pub mic: Mic,
    pub excitation: Excitation,
    /// The operator confirmed that this output will be audible.
    pub confirm_audible: bool,
}

fn sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Excitation {
    pub fn validate(&self) -> Result<()> {
        if self.kind != "log_sweep" {
            return Err("excitation kind".into());
        }
        if self.start_hz < 20
            || self.end_hz > 20000
            || self.end_hz < self.start_hz.saturating_mul(4)
            || !(8192..=32768).contains(&self.sweep_samples)
            || !TOTAL_SAMPLES.contains(&self.total_samples)
            || self.total_samples < self.sweep_samples + MIN_TAIL_SAMPLES
            || !REPEATS.contains(&self.repeats)
        {
            return Err("excitation limits".into());
        }
        if !(LEVEL_MIN_MDBFS..=LEVEL_MAX_MDBFS).contains(&self.level_mdbfs) {
            return Err("excitation level".into());
        }
        Ok(())
    }
    /// Linear peak amplitude handed to the owner (0.001..=0.0316).
    pub fn amplitude(&self) -> f64 {
        10f64.powf(f64::from(self.level_mdbfs) / 20000.)
    }
    pub fn session_samples(&self) -> usize {
        self.total_samples * usize::from(self.repeats)
    }
    /// The owner's `SweepSpec` for `sweep` / `sweep_repeated`.
    pub fn owner_spec(&self) -> serde_json::Value {
        serde_json::json!({
            "start_hz": f64::from(self.start_hz),
            "end_hz": f64::from(self.end_hz),
            "sweep_samples": self.sweep_samples,
            "total_samples": self.total_samples,
            "amplitude": self.amplitude(),
        })
    }
    /// Sample `i` of one repeat, identical to SHR PA `SweepSpec::excitation`
    /// (same expression order, so the owner's 1e-10 reference check holds).
    pub fn sample(&self, i: usize) -> f64 {
        if i >= self.sweep_samples {
            return 0.;
        }
        let start = f64::from(self.start_hz);
        let duration = self.sweep_samples as f64 / 48000.;
        let log = (f64::from(self.end_hz) / start).ln();
        let t = i as f64 / 48000.;
        let fade = (i.min(self.sweep_samples - 1 - i) as f64 / 256.).min(1.);
        self.amplitude()
            * fade
            * (2. * PI * start * duration / log * ((t * log / duration).exp() - 1.)).sin()
    }
    /// Fill a preallocated repeat buffer off the audio callback.
    pub fn fill(&self, out: &mut [f64]) -> Result<()> {
        if out.len() != self.total_samples {
            return Err("excitation buffer".into());
        }
        for (i, v) in out.iter_mut().enumerate() {
            *v = self.sample(i);
        }
        Ok(())
    }
}

/// What admission compares a plan against: the live owner state, not the plan.
pub struct Context<'a> {
    pub basis: Basis,
    pub topology: &'a EngineTopology,
    /// Output indexes assigned in the admitted PA owner graph.
    pub assigned_outputs: &'a [usize],
    /// Clock domain of the mic slot's device timeline.
    pub mic_clock_domain: &'a str,
    /// Another measurement session or owner job is active.
    pub busy: bool,
}

impl Plan {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let plan: Self = crate::show::decode_bounded(bytes, REQUEST_BYTES)?;
        plan.validate()?;
        Ok(plan)
    }
    pub fn validate(&self) -> Result<()> {
        if self.contract != CONTRACT || self.version != VERSION {
            return Err("capture contract".into());
        }
        if !identifier(&self.id)
            || !identifier(&self.position_id)
            || !identifier(&self.basis.show_id)
        {
            return Err("capture identity".into());
        }
        if [
            self.basis.source_epoch,
            self.basis.map_revision,
            self.basis.graph_generation,
            self.basis.configuration_revision,
        ]
        .iter()
        .any(|c| c.0 == 0)
        {
            return Err("capture basis".into());
        }
        if let Some(c) = &self.mic.calibration
            && (!identifier(&c.provenance) || !sha256(&c.sha256))
        {
            return Err("calibration provenance".into());
        }
        if !self.confirm_audible {
            return Err("audible output not confirmed".into());
        }
        self.excitation.validate()
    }
    /// Refusal reasons are stable wire strings.
    pub fn admit(&self, c: &Context) -> std::result::Result<(), &'static str> {
        if c.busy {
            return Err("busy");
        }
        if self.basis != c.basis {
            return Err("stale_basis");
        }
        if !c.assigned_outputs.contains(&self.path.output_index) {
            return Err("speaker_path");
        }
        let slot = self.mic.capture_slot;
        if !c.topology.measurement_slots.contains(&slot) {
            return Err("mic_slot");
        }
        // Never route the setup mic to program or monitors.
        if c.topology.inputs.iter().any(|p| p.capture_slot == slot) {
            return Err("mic_routed");
        }
        if c.mic_clock_domain != CLOCK_DOMAIN {
            return Err("clock_domain");
        }
        Ok(())
    }
}

/// Route state recorded at arm and restored exactly at every terminal state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorRoute {
    pub configuration_revision: Counter,
    /// Output mute per PA output before the session (index = output).
    pub output_muted: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    LeaseLost,
    Disconnected,
    SourceDiscontinuity,
    DeviceEpoch,
    BasisChanged,
    MicClipped,
    Deadline,
    RestoreFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Captured,
    Cancelled,
    Faulted(Fault),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Admitted; route saved; nothing audible yet.
    Armed,
    /// Excitation playing; `received` mic frames captured contiguously.
    Playing {
        first_frame: u64,
        received: usize,
    },
    /// Excitation stopped (ramped); prior route must be restored before the result.
    Restoring {
        outcome: Outcome,
    },
    Done {
        outcome: Outcome,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Source frame of excitation sample 0 of the first repeat.
    Start {
        first_frame: u64,
    },
    /// A contiguous block of mic frames with its sample peak and drop count.
    Frames {
        first_frame: u64,
        frames: usize,
        mic_peak: f32,
        dropped: u64,
    },
    Cancel,
    Fault(Fault),
    /// Elapsed control time since arm.
    Tick {
        elapsed_ms: u64,
    },
    Restored {
        ok: bool,
    },
}

pub struct Session {
    pub plan: Plan,
    pub prior: PriorRoute,
    pub phase: Phase,
    start_frame: Option<u64>,
}

impl Session {
    pub fn new(plan: Plan, prior: PriorRoute) -> Result<Self> {
        plan.validate()?;
        if prior.configuration_revision != plan.basis.configuration_revision
            || prior.output_muted.len() <= plan.path.output_index
        {
            return Err("prior route".into());
        }
        Ok(Self {
            plan,
            prior,
            phase: Phase::Armed,
            start_frame: None,
        })
    }
    pub fn deadline_ms(&self) -> u64 {
        (self.plan.excitation.session_samples() as u64 * 1000).div_ceil(u64::from(RATE))
            + DEADLINE_MARGIN_MS
    }
    /// Excitation may reach the output only while playing.
    pub fn audible(&self) -> bool {
        matches!(self.phase, Phase::Playing { .. })
    }
    fn stop(&mut self, outcome: Outcome) {
        self.phase = match self.phase {
            // Nothing played and nothing changed audibly; the saved route still
            // has to be confirmed before the result is final.
            Phase::Armed | Phase::Playing { .. } => Phase::Restoring { outcome },
            other => other,
        };
    }
    /// Apply one event. Errors are protocol misuse; faults become outcomes.
    pub fn on(&mut self, event: Event) -> std::result::Result<(), &'static str> {
        match (self.phase, event) {
            (Phase::Done { .. }, _) => Err("session finished"),
            (Phase::Restoring { outcome }, Event::Restored { ok }) => {
                self.phase = Phase::Done {
                    outcome: if ok {
                        outcome
                    } else {
                        Outcome::Faulted(Fault::RestoreFailed)
                    },
                };
                Ok(())
            }
            // Only restoration ends a stopping session; later faults are moot.
            (Phase::Restoring { .. }, _) => Ok(()),
            (_, Event::Restored { .. }) => Err("restore before stop"),
            (Phase::Armed, Event::Start { first_frame }) => {
                self.start_frame = Some(first_frame);
                self.phase = Phase::Playing {
                    first_frame,
                    received: 0,
                };
                Ok(())
            }
            (_, Event::Start { .. }) => Err("already started"),
            (_, Event::Cancel) => {
                self.stop(Outcome::Cancelled);
                Ok(())
            }
            (_, Event::Fault(f)) => {
                self.stop(Outcome::Faulted(f));
                Ok(())
            }
            (_, Event::Tick { elapsed_ms }) => {
                if elapsed_ms > self.deadline_ms() {
                    self.stop(Outcome::Faulted(Fault::Deadline));
                }
                Ok(())
            }
            (Phase::Armed, Event::Frames { .. }) => Err("frames before start"),
            (
                Phase::Playing {
                    first_frame,
                    received,
                },
                Event::Frames {
                    first_frame: at,
                    frames,
                    mic_peak,
                    dropped,
                },
            ) => {
                let total = self.plan.excitation.session_samples();
                if dropped != 0 || Some(at) != first_frame.checked_add(received as u64) {
                    self.stop(Outcome::Faulted(Fault::SourceDiscontinuity));
                } else if !mic_peak.is_finite() || mic_peak.abs() >= MIC_CLIP {
                    self.stop(Outcome::Faulted(Fault::MicClipped));
                } else {
                    let received = received.saturating_add(frames).min(total);
                    self.phase = Phase::Playing {
                        first_frame,
                        received,
                    };
                    if received == total {
                        self.stop(Outcome::Captured);
                    }
                }
                Ok(())
            }
        }
    }
    pub fn outcome(&self) -> Option<Outcome> {
        match self.phase {
            Phase::Done { outcome } => Some(outcome),
            _ => None,
        }
    }
    /// Owner `CaptureMeta` for the whole session; only a restored, complete capture
    /// has one. Repeat `n` (0-based) starts at `first_frame + n * total_samples`,
    /// which is the shared common-origin timing SHR PA `sweep_repeated` requires.
    pub fn capture(&self) -> Option<CaptureMeta> {
        let (Some(Outcome::Captured), Some(start)) = (self.outcome(), self.start_frame) else {
            return None;
        };
        let p = &self.plan;
        let b = &p.basis;
        Some(CaptureMeta {
            id: p.id.clone(),
            source_epoch: b.source_epoch,
            map_revision: b.map_revision,
            clock_domain: CLOCK_DOMAIN.into(),
            reference_id: p.id.clone(),
            reference_tap: REFERENCE_TAP.into(),
            reference_offset_frames: Counter(0),
            first_frame: Counter(start),
            sample_rate: RATE,
            output_index: p.path.output_index,
            position_id: p.position_id.clone(),
            configuration_revision: b.configuration_revision,
            dropped_frames: Counter(0),
            clipped_reference: false,
            clipped_mic: false,
            timing_verified: true,
        })
    }
    pub fn record(&self) -> Option<Record> {
        let outcome = self.outcome()?;
        Some(Record {
            contract: CONTRACT.into(),
            version: VERSION,
            id: self.plan.id.clone(),
            outcome,
            restored: outcome != Outcome::Faulted(Fault::RestoreFailed),
            prior: self.prior.clone(),
            calibration: self.plan.mic.calibration.clone(),
            owner_spec: self.plan.excitation.owner_spec(),
            repeats: self.plan.excitation.repeats,
            capture: self.capture(),
        })
    }
}

/// Exactly SHR PA `measurement::CaptureMeta` (counters as canonical strings).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureMeta {
    pub id: String,
    pub source_epoch: Counter,
    pub map_revision: Counter,
    pub clock_domain: String,
    pub reference_id: String,
    pub reference_tap: String,
    pub reference_offset_frames: Counter,
    pub first_frame: Counter,
    pub sample_rate: u32,
    pub output_index: usize,
    pub position_id: String,
    pub configuration_revision: Counter,
    pub dropped_frames: Counter,
    pub clipped_reference: bool,
    pub clipped_mic: bool,
    pub timing_verified: bool,
}

/// Terminal session result. `owner_spec` is the only floating-point part and is
/// the owner's own document; the outer contract stays integer-only on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub contract: String,
    pub version: u32,
    pub id: String,
    pub outcome: Outcome,
    pub restored: bool,
    pub prior: PriorRoute,
    pub calibration: Option<Calibration>,
    pub owner_spec: serde_json::Value,
    pub repeats: u8,
    pub capture: Option<CaptureMeta>,
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn plan() -> Plan {
        Plan {
            contract: CONTRACT.into(),
            version: VERSION,
            id: "sweep-a".into(),
            position_id: "foh-left".into(),
            basis: Basis {
                show_id: "00000000-0000-0000-0000-000000000001".into(),
                source_epoch: Counter(3),
                map_revision: Counter(1),
                graph_generation: Counter(2),
                configuration_revision: Counter(7),
            },
            path: Path { output_index: 1 },
            mic: Mic {
                capture_slot: 16,
                calibration: None,
            },
            excitation: Excitation {
                kind: "log_sweep".into(),
                start_hz: 40,
                end_hz: 16000,
                sweep_samples: 24576,
                total_samples: 32768,
                level_mdbfs: LEVEL_DEFAULT_MDBFS,
                repeats: 4,
            },
            confirm_audible: true,
        }
    }
    fn prior() -> PriorRoute {
        PriorRoute {
            configuration_revision: Counter(7),
            output_muted: vec![false, true],
        }
    }

    #[test]
    fn level_bounds_map_into_the_owner_amplitude_range() {
        let mut e = plan().excitation;
        for level in [LEVEL_MIN_MDBFS, LEVEL_DEFAULT_MDBFS, LEVEL_MAX_MDBFS] {
            e.level_mdbfs = level;
            assert!((0.001..=0.25).contains(&e.amplitude()), "{level}");
        }
        e.level_mdbfs = LEVEL_MAX_MDBFS + 1;
        assert!(e.validate().is_err());
        e.level_mdbfs = LEVEL_MIN_MDBFS - 1;
        assert!(e.validate().is_err());
    }

    #[test]
    fn excitation_limits_follow_the_owner() {
        let ok = plan().excitation;
        ok.validate().unwrap();
        let cases: [fn(&mut Excitation); 8] = [
            |e| e.kind = "noise".into(),
            |e| e.start_hz = 19,
            |e| e.end_hz = 20001,
            |e| e.end_hz = e.start_hz * 4 - 1,
            |e| e.sweep_samples = 8191,
            |e| e.total_samples = 40000,
            |e| e.sweep_samples = 32768 - MIN_TAIL_SAMPLES + 1,
            |e| e.repeats = 2,
        ];
        for case in cases {
            let mut e = ok.clone();
            case(&mut e);
            assert!(e.validate().is_err(), "{e:?}");
        }
    }

    #[test]
    fn generator_has_owner_fades_silent_tail_and_peak_level() {
        let e = plan().excitation;
        let mut buf = vec![0.; e.total_samples];
        e.fill(&mut buf).unwrap();
        assert_eq!(buf[0], 0.);
        assert!(buf[e.sweep_samples..].iter().all(|&v| v == 0.));
        let peak = buf.iter().fold(0f64, |m, v| m.max(v.abs()));
        assert!(peak <= e.amplitude() && peak > e.amplitude() * 0.99);
        assert!(e.fill(&mut buf[1..]).is_err());
    }

    #[test]
    fn plan_decode_is_strict_and_integer_only() {
        let bytes = serde_json::to_vec(&plan()).unwrap();
        assert_eq!(Plan::decode(&bytes).unwrap(), plan());
        let text = String::from_utf8(bytes).unwrap();
        for bad in [
            text.replace("-42000", "-42000.0"),
            text.replace("\"confirm_audible\":true", "\"confirm_audible\":false"),
            text.replace("\"repeats\":4", "\"repeats\":4,\"repeats\":1"),
            text.replace("\"repeats\":4", "\"repeats\":4,\"gain\":1"),
            text.replace("\"version\":1", "\"version\":2"),
            text.replace("\"source_epoch\":\"3\"", "\"source_epoch\":\"0\""),
        ] {
            assert!(Plan::decode(bad.as_bytes()).is_err(), "{bad}");
        }
        let mut p = plan();
        p.mic.calibration = Some(Calibration {
            provenance: "umik-1 7012345".into(),
            sha256: "A".repeat(64),
        });
        assert!(p.validate().is_err(), "spaces and uppercase refuse");
    }

    #[test]
    fn admission_binds_basis_path_reserved_slot_and_stagebox_clock() {
        let mut t = EngineTopology::software(16, 3, 2).unwrap();
        t.measurement_slots = vec![16];
        t.capture_channels = t.capture_channels.max(17);
        let basis = plan().basis;
        let ctx = |b: Basis, outputs: &'static [usize], clock: &'static str, busy| Context {
            basis: b,
            topology: Box::leak(Box::new(t.clone())),
            assigned_outputs: outputs,
            mic_clock_domain: clock,
            busy,
        };
        let p = plan();
        p.admit(&ctx(basis.clone(), &[0, 1], CLOCK_DOMAIN, false))
            .unwrap();
        assert_eq!(
            p.admit(&ctx(basis.clone(), &[0, 1], CLOCK_DOMAIN, true)),
            Err("busy")
        );
        let mut stale = basis.clone();
        stale.configuration_revision = Counter(8);
        assert_eq!(
            p.admit(&ctx(stale, &[0, 1], CLOCK_DOMAIN, false)),
            Err("stale_basis")
        );
        assert_eq!(
            p.admit(&ctx(basis.clone(), &[0], CLOCK_DOMAIN, false)),
            Err("speaker_path")
        );
        assert_eq!(
            p.admit(&ctx(basis.clone(), &[0, 1], "brain-local", false)),
            Err("clock_domain")
        );
        let mut q = plan();
        q.mic.capture_slot = 0;
        assert_eq!(
            q.admit(&ctx(basis, &[0, 1], CLOCK_DOMAIN, false)),
            Err("mic_slot")
        );
    }

    #[test]
    fn complete_capture_restores_then_yields_one_common_origin_capture() {
        let mut s = Session::new(plan(), prior()).unwrap();
        assert!(!s.audible());
        assert_eq!(
            s.on(Event::Frames {
                first_frame: 0,
                frames: 1,
                mic_peak: 0.,
                dropped: 0
            }),
            Err("frames before start")
        );
        s.on(Event::Start { first_frame: 96000 }).unwrap();
        assert!(s.audible());
        let total = s.plan.excitation.session_samples();
        let mut at = 96000;
        while at < 96000 + total as u64 {
            s.on(Event::Frames {
                first_frame: at,
                frames: 480,
                mic_peak: 0.25,
                dropped: 0,
            })
            .unwrap();
            at += 480;
        }
        assert!(!s.audible());
        assert!(s.capture().is_none(), "no capture before restore");
        assert!(s.record().is_none());
        s.on(Event::Restored { ok: true }).unwrap();
        let r = s.record().unwrap();
        assert_eq!(r.outcome, Outcome::Captured);
        assert!(r.restored);
        assert_eq!(r.repeats, 4);
        let c = r.capture.unwrap();
        assert_eq!(c.id, "sweep-a");
        assert_eq!(c.first_frame.0, 96000);
        assert_eq!(c.reference_tap, REFERENCE_TAP);
        assert_eq!(c.clock_domain, CLOCK_DOMAIN);
        assert!(c.timing_verified);
        assert_eq!(s.on(Event::Cancel), Err("session finished"));
    }

    #[test]
    fn cancel_faults_and_deadline_stop_audio_restore_and_yield_nothing() {
        let frames = |s: &mut Session, at, peak, dropped| {
            s.on(Event::Frames {
                first_frame: at,
                frames: 480,
                mic_peak: peak,
                dropped,
            })
        };
        let started = || {
            let mut s = Session::new(plan(), prior()).unwrap();
            s.on(Event::Start { first_frame: 0 }).unwrap();
            frames(&mut s, 0, 0.1, 0).unwrap();
            s
        };
        let mut cases: Vec<(Session, Outcome)> = Vec::new();
        let mut s = started();
        s.on(Event::Cancel).unwrap();
        cases.push((s, Outcome::Cancelled));
        let mut s = started();
        frames(&mut s, 481, 0.1, 0).unwrap();
        cases.push((s, Outcome::Faulted(Fault::SourceDiscontinuity)));
        let mut s = started();
        frames(&mut s, 480, 0.1, 1).unwrap();
        cases.push((s, Outcome::Faulted(Fault::SourceDiscontinuity)));
        let mut s = started();
        frames(&mut s, 480, 0.9995, 0).unwrap();
        cases.push((s, Outcome::Faulted(Fault::MicClipped)));
        let mut s = started();
        frames(&mut s, 480, f32::NAN, 0).unwrap();
        cases.push((s, Outcome::Faulted(Fault::MicClipped)));
        for f in [
            Fault::LeaseLost,
            Fault::Disconnected,
            Fault::DeviceEpoch,
            Fault::BasisChanged,
        ] {
            let mut s = started();
            s.on(Event::Fault(f)).unwrap();
            cases.push((s, Outcome::Faulted(f)));
        }
        let mut s = started();
        let deadline = s.deadline_ms();
        assert_eq!(deadline, 2731 + DEADLINE_MARGIN_MS);
        s.on(Event::Tick {
            elapsed_ms: deadline,
        })
        .unwrap();
        assert!(s.audible());
        s.on(Event::Tick {
            elapsed_ms: deadline + 1,
        })
        .unwrap();
        cases.push((s, Outcome::Faulted(Fault::Deadline)));
        // Cancel before any audio still restores the saved route.
        let mut s = Session::new(plan(), prior()).unwrap();
        s.on(Event::Cancel).unwrap();
        cases.push((s, Outcome::Cancelled));
        for (mut s, expected) in cases {
            assert!(!s.audible());
            assert!(s.outcome().is_none(), "restore is mandatory");
            // Late frames and faults while restoring change nothing.
            frames(&mut s, 999_999, 1.0, 9).unwrap();
            s.on(Event::Fault(Fault::Disconnected)).unwrap();
            s.on(Event::Restored { ok: true }).unwrap();
            let r = s.record().unwrap();
            assert_eq!(r.outcome, expected);
            assert!(r.restored && r.capture.is_none());
        }
    }

    #[test]
    fn failed_restore_is_its_own_fault_even_after_a_complete_capture() {
        let mut p = plan();
        p.excitation.repeats = 1;
        let mut s = Session::new(p, prior()).unwrap();
        s.on(Event::Start { first_frame: 10 }).unwrap();
        s.on(Event::Frames {
            first_frame: 10,
            frames: 32768,
            mic_peak: 0.5,
            dropped: 0,
        })
        .unwrap();
        assert_eq!(s.on(Event::Start { first_frame: 0 }), Ok(()));
        s.on(Event::Restored { ok: false }).unwrap();
        let r = s.record().unwrap();
        assert_eq!(r.outcome, Outcome::Faulted(Fault::RestoreFailed));
        assert!(!r.restored && r.capture.is_none());
    }

    #[test]
    fn prior_route_must_match_the_plan_basis_and_cover_the_path() {
        let mut bad = prior();
        bad.configuration_revision = Counter(6);
        assert!(Session::new(plan(), bad).is_err());
        let mut bad = prior();
        bad.output_muted.truncate(1);
        assert!(Session::new(plan(), bad).is_err());
    }
}
