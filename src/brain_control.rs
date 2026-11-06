//! Operator listen and held talkback contract; all runtime permission is ephemeral.
use crate::{
    control_model::{Command as AudioCommand, Request as AudioRequest, Scope},
    show::{Counter, Result},
};
use serde::{Deserialize, Serialize};
pub const HEARTBEAT_MS: u64 = 50;
pub const DEADMAN_MS: u64 = 150;
pub const FADE_FRAMES: u64 = 240;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MonitorSource {
    #[default]
    None,
    Main,
    Monitor {
        index: usize,
    },
    Pfl {
        input: usize,
    },
    Afl {
        input: usize,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Command {
    BrainSnapshot {},
    MonitorSet {
        source: MonitorSource,
        gain_cdb: i32,
        mute: bool,
        dim: bool,
        armed: bool,
    },
    TalkbackSet {
        monitors: Vec<usize>,
        gain_cdb: i32,
        mute: bool,
    },
    TalkbackFoh {
        enabled: bool,
    },
    Hold {
        generation: Counter,
    },
    Heartbeat {
        generation: Counter,
        observed_frame: Counter,
    },
    Release {
        generation: Counter,
    },
    Close {},
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub contract: String,
    pub version: u32,
    pub show_id: String,
    pub module: String,
    pub epoch: Counter,
    pub writer: Option<String>,
    pub lease: Option<Counter>,
    pub request_id: Option<Counter>,
    pub expected_revision: Option<Counter>,
    #[serde(flatten)]
    pub command: Command,
}
impl Request {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = crate::show::decode(bytes)?;
        let object = value.as_object().ok_or("brain envelope")?;
        let keys = [
            "contract",
            "version",
            "show_id",
            "module",
            "epoch",
            "writer",
            "lease",
            "request_id",
            "expected_revision",
            "kind",
            "body",
        ];
        if object.len() != keys.len() || object.keys().any(|k| !keys.contains(&k.as_str())) {
            return Err("brain envelope fields".into());
        }
        let r: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        r.validate()?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<()> {
        if self.contract != "GP15-brain" || self.version != 1 {
            return Err("brain version".into());
        }
        self.authority_request().encode()?;
        match &self.command {
            Command::MonitorSet { gain_cdb, .. } | Command::TalkbackSet { gain_cdb, .. }
                if !(-9000..=0).contains(gain_cdb) =>
            {
                Err("brain gain range".into())
            }
            Command::TalkbackSet { monitors, .. } if monitors.len() > 4096 => {
                Err("brain destination bound".into())
            }
            Command::Hold { generation }
            | Command::Heartbeat { generation, .. }
            | Command::Release { generation }
                if generation.0 == 0 =>
            {
                Err("hold generation".into())
            }
            _ => Ok(()),
        }
    }
    pub fn authority_request(&self) -> AudioRequest {
        AudioRequest {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: self.show_id.clone(),
            module: self.module.clone(),
            epoch: self.epoch,
            writer: self.writer.clone(),
            lease: self.lease,
            request_id: self.request_id,
            expected_revision: self.expected_revision,
            command: if matches!(self.command, Command::BrainSnapshot {}) {
                AudioCommand::Snapshot {}
            } else {
                AudioCommand::Renew {}
            },
        }
    }
    pub fn scope(&self) -> Scope {
        match self.command {
            Command::MonitorSet { .. } | Command::BrainSnapshot {} => Scope::LocalOperatorMonitor,
            Command::TalkbackFoh { .. } => Scope::TalkbackFoh,
            _ => Scope::TalkbackDestinations,
        }
    }
    pub fn fingerprint(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        Ok(crate::hash::hex(Sha256::digest(
            serde_json::to_vec(self).map_err(|e| e.to_string())?,
        )))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub source: MonitorSource,
    pub selection_generation: Counter,
    pub monitor_gain_cdb: i32,
    pub monitor_mute: bool,
    pub monitor_dim: bool,
    pub monitor_armed: bool,
    pub talkback_monitors: Vec<usize>,
    pub talkback_foh: bool,
    pub talkback_gain_cdb: i32,
    pub talkback_mute: bool,
    pub hold_generation_counter: Counter,
    pub held_generation: Option<Counter>,
    pub hold_deadline_ms: Option<Counter>,
    pub audible_path_ready: bool,
    pub talkback_path_ready: bool,
    pub monitor_path_ready: bool,
    /// Linear peak amplitude in billionths of digital full scale (1.0 = 1_000_000_000).
    pub microphone_peak_nano: u64,
    pub outgoing_peak_nano: u64,
    pub monitor_peak_nano: u64,
    pub frame: Counter,
    pub revision: Counter,
    pub heartbeat_ms: u64,
    pub deadman_ms: u64,
    pub fade_frames: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub contract: String,
    pub version: u32,
    pub state: String,
    pub reason: Option<String>,
    pub context: crate::mixer_control::RequestContext,
    pub revision: Counter,
    pub applied_frame: Option<Counter>,
    pub snapshot: Option<Snapshot>,
}
impl Reply {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let value: Self = crate::show::decode(bytes)?;
        if value.contract != "GP15-brain" || value.version != 1 {
            return Err("brain reply version".into());
        }
        value.context.validate()?;
        Ok(value)
    }
    pub fn new(
        r: &Request,
        state: &str,
        reason: Option<String>,
        revision: Counter,
        frame: Option<u64>,
        snapshot: Option<Snapshot>,
    ) -> Self {
        Self {
            contract: "GP15-brain".into(),
            version: 1,
            state: state.into(),
            reason,
            context: crate::mixer_control::RequestContext::request(&r.authority_request()),
            revision,
            applied_frame: frame.map(Counter),
            snapshot,
        }
    }
}
#[derive(Clone)]
pub(crate) struct State {
    pub source: MonitorSource,
    pub selection_generation: u64,
    pub monitor_gain_cdb: i32,
    pub monitor_mute: bool,
    pub monitor_dim: bool,
    pub monitor_armed: bool,
    pub monitors: Vec<usize>,
    pub destination_hash: [u8; 32],
    pub foh: bool,
    pub foh_authority: Option<AudioRequest>,
    pub gain_cdb: i32,
    pub mute: bool,
    pub hold: Option<(AudioRequest, u64, u64, u64)>,
    pub high_generation: u64,
    pub envelope: f64,
    pub microphone_peak: f64,
    pub outgoing_peak: f64,
    pub monitor_peak: f64,
    pub monitor_envelope: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            source: MonitorSource::None,
            selection_generation: 1,
            monitor_gain_cdb: -1200,
            monitor_mute: true,
            monitor_dim: false,
            monitor_armed: false,
            monitors: Vec::new(),
            destination_hash: crate::held_proof::destination_hash(&[])
                .expect("empty destination set"),
            foh: false,
            foh_authority: None,
            gain_cdb: -1200,
            mute: true,
            hold: None,
            high_generation: 0,
            envelope: 0.,
            microphone_peak: 0.,
            outgoing_peak: 0.,
            monitor_peak: 0.,
            monitor_envelope: 0.,
        }
    }
}
impl State {
    pub fn close(&mut self) {
        self.hold = None;
    }
    pub fn fault(&mut self) {
        self.close();
        if self.monitor_armed {
            self.selection_generation = self.selection_generation.saturating_add(1);
        }
        self.monitor_armed = false;
        self.monitor_envelope = 0.;
        self.envelope = 0.;
    }
    pub fn apply(
        &mut self,
        r: &Request,
        now: u64,
        frame: u64,
        inputs: usize,
        monitors: usize,
    ) -> Result<()> {
        match &r.command {
            Command::BrainSnapshot {} => (),
            Command::MonitorSet {
                source,
                gain_cdb,
                mute,
                dim,
                armed,
            } => {
                if matches!(source,MonitorSource::Monitor{index} if *index>=monitors)
                    || matches!(source,MonitorSource::Pfl{input}|MonitorSource::Afl{input} if *input>=inputs)
                {
                    return Err("monitor source range".into());
                }
                if self.source != *source && *armed {
                    return Err("source change requires disarmed readback then rearm".into());
                }
                if self.source != *source || (self.monitor_armed && !*armed) {
                    self.selection_generation = self
                        .selection_generation
                        .checked_add(1)
                        .ok_or("selection exhausted")?;
                    self.monitor_envelope = 0.;
                }
                if *armed && self.selection_generation == u64::MAX {
                    return Err("selection exhausted".into());
                }
                self.source = *source;
                self.monitor_gain_cdb = *gain_cdb;
                self.monitor_mute = *mute;
                self.monitor_dim = *dim;
                self.monitor_armed = *armed;
            }
            Command::TalkbackSet {
                monitors: selected,
                gain_cdb,
                mute,
            } => {
                if selected.iter().any(|i| *i >= monitors)
                    || selected
                        .iter()
                        .enumerate()
                        .any(|(i, v)| selected[..i].contains(v))
                {
                    return Err("talkback destination range/duplicate".into());
                }
                self.close();
                self.monitors = selected.clone();
                self.gain_cdb = *gain_cdb;
                self.mute = *mute;
            }
            Command::TalkbackFoh { enabled } => {
                self.close();
                self.foh = *enabled;
                self.foh_authority = enabled.then(|| r.authority_request());
            }
            Command::Hold { generation } => {
                if generation.0 <= self.high_generation
                    || self.hold.is_some()
                    || self.mute
                    || (self.monitors.is_empty() && !self.foh)
                {
                    return Err("hold closed/generation/owner".into());
                }
                self.high_generation = generation.0;
                self.hold = Some((
                    r.authority_request(),
                    generation.0,
                    now.checked_add(DEADMAN_MS).ok_or("deadline exhausted")?,
                    frame
                        .checked_add(DEADMAN_MS * 48)
                        .ok_or("frame exhausted")?,
                ));
            }
            Command::Heartbeat {
                generation,
                observed_frame,
            } => {
                let Some((owner, g, deadline, frame_deadline)) = &mut self.hold else {
                    return Err("hold released".into());
                };
                if *g != generation.0
                    || owner.writer != r.writer
                    || owner.lease != r.lease
                    || now >= *deadline
                    || frame >= *frame_deadline
                {
                    return Err("hold stale/owner".into());
                }
                if observed_frame.0 > frame || frame - observed_frame.0 > HEARTBEAT_MS * 48 {
                    return Err("heartbeat observed frame stale/future".into());
                }
                let absolute = observed_frame
                    .0
                    .checked_add(DEADMAN_MS * 48)
                    .ok_or("frame exhausted")?;
                if absolute < *frame_deadline {
                    return Err("heartbeat observed frame regressed".into());
                }
                *deadline = now
                    .checked_add((absolute - frame) / 48)
                    .ok_or("deadline exhausted")?;
                *frame_deadline = absolute;
            }
            Command::Release { generation } => {
                if self
                    .hold
                    .as_ref()
                    .is_some_and(|(o, g, _, _)| o.writer != r.writer || *g != generation.0)
                {
                    return Err("hold owner/generation".into());
                }
                self.high_generation = self.high_generation.max(generation.0);
                self.close();
            }
            Command::Close {} => self.close(),
        }
        Ok(())
    }
    pub fn snapshot(&self, frame: u64, revision: Counter, ready: bool) -> Snapshot {
        Snapshot {
            source: self.source,
            selection_generation: Counter(self.selection_generation),
            monitor_gain_cdb: self.monitor_gain_cdb,
            monitor_mute: self.monitor_mute,
            monitor_dim: self.monitor_dim,
            monitor_armed: self.monitor_armed,
            talkback_monitors: self.monitors.clone(),
            talkback_foh: self.foh,
            talkback_gain_cdb: self.gain_cdb,
            talkback_mute: self.mute,
            hold_generation_counter: Counter(self.high_generation),
            held_generation: self.hold.as_ref().map(|h| Counter(h.1)),
            hold_deadline_ms: self.hold.as_ref().map(|h| Counter(h.2)),
            audible_path_ready: ready,
            talkback_path_ready: false,
            monitor_path_ready: false,
            microphone_peak_nano: (self.microphone_peak * 1_000_000_000.).round() as u64,
            outgoing_peak_nano: (self.outgoing_peak * 1_000_000_000.).round() as u64,
            monitor_peak_nano: (self.monitor_peak * 1_000_000_000.).round() as u64,
            frame: Counter(frame),
            revision,
            heartbeat_ms: HEARTBEAT_MS,
            deadman_ms: DEADMAN_MS,
            fade_frames: FADE_FRAMES,
        }
    }
}

/// Stable settings only. No hold, grant, live owner, epoch or output arming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub version: u32,
    pub source: MonitorSource,
    pub monitor_gain_cdb: i32,
    pub monitor_mute: bool,
    pub monitor_dim: bool,
    pub talkback_monitors: Vec<usize>,
    pub talkback_gain_cdb: i32,
    pub talkback_mute: bool,
}
impl Intent {
    pub fn validate(&self, topology: &crate::topology::EngineTopology) -> Result<()> {
        if self.version != 1
            || !(-9000..=0).contains(&self.monitor_gain_cdb)
            || !(-9000..=0).contains(&self.talkback_gain_cdb)
            || matches!(self.source,MonitorSource::Monitor{index} if index>=topology.monitors)
            || matches!(self.source,MonitorSource::Pfl{input}|MonitorSource::Afl{input} if input>=topology.inputs.len())
            || self.talkback_monitors.len() > topology.monitors
            || self
                .talkback_monitors
                .iter()
                .enumerate()
                .any(|(n, i)| *i >= topology.monitors || self.talkback_monitors[..n].contains(i))
        {
            return Err("brain intent mismatch".into());
        }
        Ok(())
    }
}
impl State {
    pub fn intent(&self) -> Intent {
        Intent {
            version: 1,
            source: self.source,
            monitor_gain_cdb: self.monitor_gain_cdb,
            monitor_mute: self.monitor_mute,
            monitor_dim: self.monitor_dim,
            talkback_monitors: self.monitors.clone(),
            talkback_gain_cdb: self.gain_cdb,
            talkback_mute: self.mute,
        }
    }
    pub fn from_intent(intent: &Intent) -> Self {
        Self {
            source: intent.source,
            monitor_gain_cdb: intent.monitor_gain_cdb,
            monitor_mute: intent.monitor_mute,
            monitor_dim: intent.monitor_dim,
            monitors: intent.talkback_monitors.clone(),
            destination_hash: crate::held_proof::destination_hash(&intent.talkback_monitors)
                .expect("validated persisted destinations"),
            gain_cdb: intent.talkback_gain_cdb,
            mute: intent.talkback_mute,
            ..Self::default()
        }
    }
}

/// Borrowed, already-authorized render view. No strings, leases or heap owners
/// may be retired by the sample renderer. The controller supplies the exact
/// consumer deadline, validated destinations, prepared gain and output-safety ramp.
pub struct TalkbackRender<'a> {
    pub first_frame: u64,
    pub deadline_frame: Option<u64>,
    pub monitors: &'a [usize],
    pub foh: bool,
    pub gain: f64,
    pub safety: &'a [f64; 48],
}
/// Production talkback gain/fade/injection render section. Returns digital
/// microphone and injected peaks. No allocation/free, locks, I/O or coefficient
/// design. Resource bound is the existing16-bit route index wire limit; topology resource
/// admission remains independently narrower and can evolve with the protocol.
pub fn render_talkback_block(
    view: &TalkbackRender<'_>,
    envelope: &mut f64,
    samples: Option<&[f64]>,
    output: &mut [f64],
    buses: usize,
) -> std::result::Result<(f64, f64), &'static str> {
    let maximum_buses = usize::from(u16::MAX) + 2;
    if !(2..=maximum_buses).contains(&buses)
        || output.len() != 48 * buses
        || view.monitors.len() > buses - 2
    {
        return Err("talkback render shape");
    }
    output.fill(0.);
    if view.first_frame.checked_add(48).is_none()
        || !view.gain.is_finite()
        || !(0.0..=1.0).contains(&view.gain)
        || !envelope.is_finite()
        || !(0.0..=1.0).contains(envelope)
        || view
            .safety
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        || view.monitors.iter().any(|&i| i >= buses - 2)
        || samples.is_some_and(|v| v.len() != 48 || v.iter().any(|s| !s.is_finite()))
    {
        return Err("talkback render input");
    }
    let mut microphone_peak = 0_f64;
    let mut outgoing_peak = 0_f64;
    for f in 0..48 {
        let active = samples.is_some()
            && view
                .deadline_frame
                .is_some_and(|deadline| view.first_frame + (f as u64) < deadline);
        let target = if active { 1. } else { 0. };
        *envelope += (target - *envelope).clamp(-1. / FADE_FRAMES as f64, 1. / FADE_FRAMES as f64);
        let raw = samples.map_or(0., |v| v[f]);
        microphone_peak = microphone_peak.max(raw.abs());
        let value = raw.clamp(-1., 1.) * view.gain * *envelope * view.safety[f];
        outgoing_peak = outgoing_peak.max(value.abs());
        for &index in view.monitors {
            output[f * buses + 2 + index] = value;
        }
        if view.foh {
            output[f * buses] = value * std::f64::consts::FRAC_1_SQRT_2;
            output[f * buses + 1] = value * std::f64::consts::FRAC_1_SQRT_2;
        }
    }
    Ok((microphone_peak, outgoing_peak))
}

#[cfg(test)]
mod held_proof_intent_tests {
    use super::*;
    #[test]
    fn held_proof_restores_admitted_intent_and_hash_keeps_command_limit_separate() {
        // Validate metadata with an explicit resource budget without allocating
        // mixer buffers. Coherent snapshot admission remains one MiB.
        let mut topology = crate::topology::EngineTopology::software(1, 0, 0).unwrap();
        topology.monitors = 853;
        let admission = topology
            .validate(crate::topology::ResourceBudget {
                bytes: 512 * 1024 * 1024,
                sample_operations: 32 * 1024 * 1024,
            })
            .unwrap();
        assert_eq!(admission.estimated_snapshot_bytes, 1_047_680);
        let mut intent = State::default().intent();
        intent.talkback_monitors = (0..853).collect();
        intent.validate(&topology).unwrap();
        let restored = State::from_intent(&intent);
        assert_eq!(restored.monitors, intent.talkback_monitors);
        assert_eq!(
            restored.destination_hash,
            crate::held_proof::destination_hash(&intent.talkback_monitors).unwrap()
        );
        // This helper is not the command validator. Its encoding supports larger
        // sets, but current coherent snapshot admission does NOT admit a 4097-
        // monitor topology; do not claim such an intent is presently loadable.
        let larger: Vec<usize> = (0..4097).collect();
        assert!(crate::held_proof::destination_hash(&larger).is_ok());
        topology.monitors = 4097;
        assert!(
            topology
                .validate(crate::topology::ResourceBudget {
                    bytes: 512 * 1024 * 1024,
                    sample_operations: 32 * 1024 * 1024,
                })
                .unwrap_err()
                .contains("snapshot admission")
        );
        // Topology::validate rejects >u16 monitors; validated intent indices are
        // less than that monitor count, so all fit the canonical u32 encoding.
        topology.monitors = usize::from(u16::MAX) + 1;
        assert!(
            topology
                .validate(crate::topology::ResourceBudget {
                    bytes: usize::MAX,
                    sample_operations: usize::MAX
                })
                .is_err()
        );
    }
}
