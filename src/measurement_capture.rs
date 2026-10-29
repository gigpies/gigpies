//! Bounded software-only source-pair capture; the PA executable owns all analysis.
use crate::{
    measurement_owner::{self, Payload},
    measurement_wire::{Basis, Command, Options, Record, Summary},
    show::{Counter, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::VecDeque;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bin {
    frequency_hz: f64,
    magnitude: f64,
    phase_radians: f64,
    coherence: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerOptions {
    max_arrival_samples: usize,
    band_hz: [f64; 2],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Measurement {
    contract: String,
    version: u32,
    algorithm: String,
    capture: Meta,
    options: OwnerOptions,
    samples: usize,
    arrival_samples: i32,
    signed_correlation: f64,
    competing_peak_ratio: f64,
    segments: usize,
    spectrum: Vec<Bin>,
    proposal_eligible: bool,
    reasons: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Refusal {
    contract: String,
    version: u32,
    status: String,
    reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Change {
    output_index: usize,
    before_delay_samples: u32,
    after_delay_samples: u32,
    before_inverted: bool,
    after_inverted: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    contract: String,
    version: u32,
    status: String,
    basis_configuration_revision: Counter,
    basis_capture: Meta,
    basis_configuration: Value,
    measurement_ids: Vec<String>,
    changes: Vec<Change>,
    added_latency_samples: u32,
    verification_required: bool,
    reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    contract: String,
    version: u32,
    basis_capture: Meta,
    basis_configuration_revision: Counter,
    configuration_json: String,
}
#[derive(Clone)]
struct Guard {
    auth: crate::control_model::Request,
    basis: Basis,
}
pub struct Capture {
    meta: Meta,
    options: Options,
    reference_slot: usize,
    mic_slot: usize,
    channels: usize,
    samples: usize,
    reference: Vec<f64>,
    mic: Vec<f64>,
    next: u64,
    failed: Option<&'static str>,
    deadline: u64,
}
pub enum Action {
    Start(Box<Capture>),
    Cancel(String),
    Propose {
        id: String,
        request: Value,
        candidate: (Value, String),
        ids: Vec<String>,
        meta: Box<Meta>,
    },
    Failed(String),
}
struct Waiting {
    id: String,
    token: u64,
    deadline: std::time::Instant,
    expect: Expected,
}
enum Expected {
    Measurement {
        meta: Meta,
        options: Options,
        samples: usize,
    },
    Proposal {
        ids: Vec<String>,
        meta: Meta,
        operation: String,
        target: Option<String>,
    },
}
pub struct State {
    worker: measurement_owner::Worker,
    pub records: VecDeque<Record>,
    guards: VecDeque<(String, Guard)>,
    capture: Option<Box<Capture>>,
    waiting: Option<Waiting>,
}
impl State {
    pub fn new(config: measurement_owner::Config) -> Result<Self> {
        Ok(Self {
            worker: measurement_owner::Worker::start(config)?,
            records: VecDeque::with_capacity(16),
            guards: VecDeque::with_capacity(16),
            capture: None,
            waiting: None,
        })
    }
    pub fn busy(&self) -> bool {
        self.capture.is_some() || self.waiting.is_some()
    }
    pub fn progress(&self) -> Option<crate::measurement_wire::Progress> {
        self.capture
            .as_ref()
            .map(|c| crate::measurement_wire::Progress {
                id: c.meta.id.clone(),
                received_samples: c.reference.len(),
                requested_samples: c.samples,
                output_index: c.meta.output_index,
            })
    }
    pub fn active_basis(&self) -> Option<&Basis> {
        let id = self.active_id()?;
        self.guards
            .iter()
            .find(|(old, _)| old == id)
            .map(|(_, g)| &g.basis)
    }
    pub fn active_id(&self) -> Option<&str> {
        self.capture
            .as_ref()
            .map(|c| c.meta.id.as_str())
            .or_else(|| self.waiting.as_ref().map(|w| w.id.as_str()))
    }
    pub fn guards(&self) -> impl Iterator<Item = (&str, &crate::control_model::Request, &Basis)> {
        self.guards
            .iter()
            .map(|(id, g)| (id.as_str(), &g.auth, &g.basis))
    }
    pub fn authorize_records(
        &self,
        command: &Command,
        auth: &crate::control_model::Request,
    ) -> Result<()> {
        let ids: Vec<&str> = match command {
            Command::CaptureCancel { id } => vec![id],
            Command::MeasurementEq { captures, .. } => {
                captures.iter().map(String::as_str).collect()
            }
            Command::MeasurementPropose { pairs, .. }
            | Command::MeasurementOverlap { pairs, .. } => {
                pairs.iter().flatten().map(String::as_str).collect()
            }
            _ => Vec::new(),
        };
        for id in ids {
            let guard = self
                .guards
                .iter()
                .find(|(old, _)| old == id)
                .ok_or("measurement authority missing")?;
            if guard.1.auth.writer != auth.writer || guard.1.auth.lease != auth.lease {
                return Err("measurement belongs to another writer lease".into());
            }
        }
        Ok(())
    }
    pub fn prepare(
        &self,
        command: &Command,
        basis: &Basis,
        topology: &crate::topology::EngineTopology,
        frame: u64,
        now: u64,
    ) -> Result<Action> {
        match command {
            Command::CaptureCancel { id } => {
                if self.active_id() == Some(id) {
                    Ok(Action::Cancel(id.clone()))
                } else {
                    Err("no matching active measurement".into())
                }
            }
            Command::CaptureStart {
                id,
                reference_input,
                mic_capture_slot,
                output_index,
                position_id,
                samples,
                options,
            } => {
                if !(32768..=65536).contains(samples)
                    || !(1..=2048).contains(&options.max_arrival_samples)
                    || !crate::measurement_wire::identifier(id)
                    || !crate::measurement_wire::identifier(position_id)
                {
                    return Err("capture bounds".into());
                }
                if self.busy() {
                    return Err("measurement busy".into());
                }
                if self.records.iter().any(|r| r.summary.id == *id) {
                    return Err("measurement ID already retained".into());
                }
                if !topology.measurement_slots.contains(mic_capture_slot)
                    || topology
                        .inputs
                        .iter()
                        .any(|p| p.capture_slot == *mic_capture_slot)
                {
                    return Err("setup microphone must use reserved non-program slot".into());
                }
                let reference_slot = topology
                    .inputs
                    .iter()
                    .find(|p| p.id == *reference_input)
                    .ok_or("reference input identity")?
                    .capture_slot;
                let graph: Value = measurement_owner::decode_document(
                    basis.configuration_json.as_bytes(),
                    48 * 1024,
                )?;
                let output = graph["outputs"]
                    .as_array()
                    .and_then(|v| v.get(*output_index))
                    .ok_or("measurement output index")?;
                if output["source"].is_null() {
                    return Err("measurement output unassigned".into());
                }
                let meta = Meta {
                    id: id.clone(),
                    source_epoch: basis.source_epoch,
                    map_revision: basis.map_revision,
                    clock_domain: basis.clock_domain.clone(),
                    reference_id: reference_input.clone(),
                    reference_tap: "raw-pre-fader".into(),
                    reference_offset_frames: Counter(0),
                    first_frame: Counter(frame),
                    sample_rate: 48000,
                    output_index: *output_index,
                    position_id: position_id.clone(),
                    configuration_revision: basis.graph_generation,
                    dropped_frames: Counter(0),
                    clipped_reference: false,
                    clipped_mic: false,
                    timing_verified: true,
                };
                Ok(Action::Start(Box::new(Capture {
                    meta,
                    options: options.clone(),
                    reference_slot,
                    mic_slot: *mic_capture_slot,
                    channels: topology.capture_channels,
                    samples: *samples,
                    reference: Vec::with_capacity(*samples),
                    mic: Vec::with_capacity(*samples),
                    next: frame,
                    failed: None,
                    deadline: now.saturating_add(5000),
                })))
            }
            Command::MeasurementEq {
                id,
                captures,
                target,
            } => {
                if self.busy() || self.records.iter().any(|r| r.summary.id == *id) {
                    return Err("measurement busy or retained ID".into());
                }
                let mut positions: Vec<Value> = Vec::new();
                for id in captures {
                    let r = self
                        .records
                        .iter()
                        .find(|r| r.summary.id == *id)
                        .ok_or("measurement result missing")?;
                    if r.summary.state != "measured" || &r.basis != basis {
                        return Err("stale or unavailable measurement result".into());
                    }
                    positions.push(measurement_owner::decode_document(
                        r.owner_result_json
                            .as_ref()
                            .ok_or("measurement missing")?
                            .as_bytes(),
                        65536,
                    )?);
                }
                let configuration = measurement_owner::decode_document(
                    basis.configuration_json.as_bytes(),
                    48 * 1024,
                )?;
                let meta = serde_json::from_value::<Meta>(
                    positions.first().ok_or("empty positions")?["capture"].clone(),
                )
                .map_err(|e| e.to_string())?;
                Ok(Action::Propose {
                    id: id.clone(),
                    request: json!({"contract":"C-PA-MEASUREMENT","version":2,"operation":"eq_propose","configuration_revision":basis.graph_generation,"configuration":configuration,"positions":positions,"target":target}),
                    candidate: (configuration, basis.graph_generation.0.to_string()),
                    ids: captures.clone(),
                    meta: Box::new(meta),
                })
            }
            Command::MeasurementPropose { id, pairs }
            | Command::MeasurementOverlap { id, pairs } => {
                if self.busy() {
                    return Err("measurement busy".into());
                }
                if self.records.iter().any(|r| r.summary.id == *id) {
                    return Err("measurement ID already retained".into());
                }
                let mut positions = Vec::with_capacity(pairs.len());
                let mut ids = Vec::new();
                let mut first = None;
                for pair in pairs {
                    let mut values = Vec::new();
                    for id in pair {
                        let r = self
                            .records
                            .iter()
                            .find(|r| r.summary.id == *id)
                            .ok_or("measurement result missing")?;
                        if r.summary.state != "measured" || &r.basis != basis {
                            return Err("stale or unavailable measurement result".into());
                        }
                        let v: Value = measurement_owner::decode_document(
                            r.owner_result_json
                                .as_ref()
                                .ok_or("measurement result missing")?
                                .as_bytes(),
                            65536,
                        )?;
                        if first.is_none() {
                            first = Some(
                                serde_json::from_value::<Meta>(v["capture"].clone())
                                    .map_err(|e| e.to_string())?,
                            )
                        }
                        values.push(v);
                        ids.push(id.clone());
                    }
                    positions.push(json!({"anchor":values[0],"target":values[1]}));
                }
                let configuration: Value = measurement_owner::decode_document(
                    basis.configuration_json.as_bytes(),
                    48 * 1024,
                )?;
                Ok(Action::Propose {
                    id: id.clone(),
                    request: json!({"contract":"C-PA-MEASUREMENT","version":if matches!(command, Command::MeasurementOverlap{..}) {2} else {1},"operation":if matches!(command, Command::MeasurementOverlap{..}) {"overlap"} else {"propose"},"configuration_revision":basis.graph_generation,"configuration":configuration,"positions":positions}),
                    candidate: (configuration, basis.graph_generation.0.to_string()),
                    ids,
                    meta: Box::new(first.ok_or("empty positions")?),
                })
            }
            _ => Err("measurement action".into()),
        }
    }
    fn insert(
        &mut self,
        id: String,
        kind: &str,
        auth: crate::control_model::Request,
        basis: Basis,
    ) {
        if self.records.len() == 16
            && let Some(old) = self.records.pop_front()
        {
            self.guards.retain(|(id, _)| *id != old.summary.id);
        }
        self.guards.push_back((
            id.clone(),
            Guard {
                auth,
                basis: basis.clone(),
            },
        ));
        self.records.push_back(Record {
            summary: Summary {
                id,
                kind: kind.into(),
                state: "capturing".into(),
                reason: None,
            },
            basis,
            owner_result_json: None,
            candidate_configuration_json: None,
        });
    }
    pub fn apply(
        &mut self,
        action: Action,
        auth: crate::control_model::Request,
        basis: Basis,
    ) -> Result<()> {
        match action {
            Action::Start(c) => {
                if self.busy() {
                    return Err("measurement busy".into());
                }
                self.insert(c.meta.id.clone(), "capture", auth, basis);
                self.capture = Some(c);
                Ok(())
            }
            Action::Cancel(id) => {
                self.invalidate(&id, "cancelled");
                Ok(())
            }
            Action::Propose {
                id,
                request,
                candidate,
                ids,
                meta,
            } => {
                if self.busy() {
                    return Err("measurement busy".into());
                }
                let operation = request["operation"]
                    .as_str()
                    .ok_or("owner operation")?
                    .to_string();
                let target = request["target"].as_str().map(String::from);
                let token = self.worker.token();
                self.worker.submit(measurement_owner::Job {
                    token,
                    request: Payload::Json(request),
                    candidate: Some(candidate),
                })?;
                self.insert(id.clone(), "proposal", auth, basis);
                self.record_mut(&id).unwrap().summary.state = "analyzing".into();
                self.waiting = Some(Waiting {
                    id,
                    token,
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(12),
                    expect: Expected::Proposal {
                        ids,
                        meta: *meta,
                        operation,
                        target,
                    },
                });
                Ok(())
            }
            Action::Failed(reason) => Err(reason),
        }
    }
    pub fn invalidate_prepared(
        &mut self,
        action: Action,
        auth: crate::control_model::Request,
        basis: Basis,
        reason: &str,
    ) {
        let (id, kind) = match action {
            Action::Start(c) => (c.meta.id, "capture"),
            Action::Propose { id, .. } => (id, "proposal"),
            _ => return,
        };
        self.insert(id.clone(), kind, auth, basis);
        self.invalidate(&id, reason);
    }
    fn record_mut(&mut self, id: &str) -> Option<&mut Record> {
        self.records.iter_mut().find(|r| r.summary.id == id)
    }
    pub fn invalidate(&mut self, id: &str, reason: &str) {
        if self.active_id() == Some(id) {
            self.worker.cancel();
            self.capture = None;
            self.waiting = None;
        }
        if let Some(r) = self.record_mut(id) {
            r.summary.state = if reason == "cancelled" {
                "cancelled"
            } else {
                "invalidated"
            }
            .into();
            r.summary.reason = Some(reason.into());
            r.candidate_configuration_json = None;
        }
        self.guards.retain(|(old, _)| old != id);
    }
    pub fn invalidate_all(&mut self, reason: &str) {
        let ids: Vec<_> = self.guards.iter().map(|(id, _)| id.clone()).collect();
        for id in ids {
            self.invalidate(&id, reason);
        }
    }
    /// Only bounded copies/flags. No allocation, lock, spawn, I/O or buffer retirement.
    pub fn offer(&mut self, epoch: u64, map: u64, first: u64, raw: &[f64], basis_valid: bool) {
        let Some(c) = self.capture.as_mut() else {
            return;
        };
        if c.failed.is_some() || c.reference.len() == c.samples {
            return;
        }
        if !basis_valid
            || epoch != c.meta.source_epoch.0
            || map != c.meta.map_revision.0
            || first != c.next
            || raw.len() != 48 * c.channels
        {
            c.failed = Some("capture source discontinuity");
            return;
        }
        let count = (c.samples - c.reference.len()).min(48);
        for frame in 0..count {
            let a = raw[frame * c.channels + c.reference_slot];
            let b = raw[frame * c.channels + c.mic_slot];
            if !a.is_finite() || !b.is_finite() || a.abs() >= 1. || b.abs() >= 1. {
                c.failed = Some("capture clipped or nonfinite");
                return;
            }
            c.reference.push(a);
            c.mic.push(b);
        }
        c.next = c.next.saturating_add(48);
    }
    /// Called only in the control pump; retires buffers and observes worker completions.
    pub fn poll(&mut self, now: u64) {
        if self
            .waiting
            .as_ref()
            .is_some_and(|w| std::time::Instant::now() > w.deadline)
        {
            let id = self.waiting.as_ref().unwrap().id.clone();
            self.invalidate(&id, "owner completion deadline");
        }

        if let Some(c) = &self.capture {
            if c.failed.is_some() || now > c.deadline {
                let id = c.meta.id.clone();
                let reason = c.failed.unwrap_or("capture deadline");
                self.invalidate(&id, reason);
            } else if c.reference.len() == c.samples {
                let c = self.capture.take().unwrap();
                let id = c.meta.id.clone();
                let token = self.worker.token();
                let expected = Expected::Measurement {
                    meta: c.meta.clone(),
                    options: c.options.clone(),
                    samples: c.samples,
                };
                let submitted = self.worker.submit(measurement_owner::Job {
                    token,
                    request: Payload::Samples {
                        capture: json!(c.meta),
                        options: c.options,
                        reference: c.reference,
                        mic: c.mic,
                    },
                    candidate: None,
                });
                match submitted {
                    Ok(()) => {
                        self.record_mut(&id).unwrap().summary.state = "analyzing".into();
                        self.waiting = Some(Waiting {
                            id,
                            token,
                            deadline: std::time::Instant::now()
                                + std::time::Duration::from_secs(12),
                            expect: expected,
                        });
                    }
                    Err(e) => self.invalidate(&id, &e),
                }
            }
        }
        while let Some(done) = self.worker.poll() {
            let Some(w) = self.waiting.as_ref() else {
                continue;
            };
            if done.token != w.token {
                continue;
            }
            let w = self.waiting.take().unwrap();
            let result = done.result.and_then(|(v, candidate)| {
                validate_result(
                    &w.expect,
                    self.records
                        .iter()
                        .find(|r| r.summary.id == w.id)
                        .ok_or("result record missing")?,
                    v,
                    candidate,
                )
            });
            match result {
                Ok((state, reason, value, candidate)) => {
                    if let Some(r) = self.record_mut(&w.id) {
                        r.summary.state = state;
                        r.summary.reason = reason;
                        r.owner_result_json =
                            Some(serde_json::to_string(&value).expect("validated finite JSON"));
                        r.candidate_configuration_json = candidate;
                    }
                }
                Err(e) => self.invalidate(&w.id, &e),
            }
        }
    }
}
fn validate_result(
    expected: &Expected,
    record: &Record,
    value: Value,
    candidate: Option<Value>,
) -> Result<(String, Option<String>, Value, Option<String>)> {
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 65536 {
        return Err("owner JSON result capacity".into());
    }
    if value.get("contract").and_then(Value::as_str) == Some("C-PA-MEASUREMENT-REFUSAL") {
        let r: Refusal = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        if r.version != 1 || r.status != "refused" || r.reason.len() > 1024 || candidate.is_some() {
            return Err("owner refusal schema".into());
        }
        return Ok(("refused".into(), Some(r.reason), value, None));
    }
    match expected {
        Expected::Measurement {
            meta,
            options,
            samples,
        } => {
            let m: Measurement =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            if m.contract != "C-PA-MEASUREMENT-RESULT"
                || m.version != 1
                || m.algorithm != "offline-h1-v1"
                || &m.capture != meta
                || m.options.max_arrival_samples != options.max_arrival_samples
                || m.options.band_hz != options.band_hz.map(f64::from)
                || m.samples != *samples
                || m.arrival_samples.unsigned_abs() as usize > options.max_arrival_samples
                || !m.signed_correlation.is_finite()
                || m.signed_correlation.abs() > 1.000001
                || !m.competing_peak_ratio.is_finite()
                || m.competing_peak_ratio < 0.
                || m.segments
                    != (m.samples - m.arrival_samples.unsigned_abs() as usize - 4096) / 2048 + 1
                || !(1..=64).contains(&m.spectrum.len())
                || (m.proposal_eligible && m.spectrum.len() < 8)
                || m.reasons.len() > 8
                || m.reasons.iter().any(|s| s.len() > 128)
                || m.proposal_eligible != m.reasons.is_empty()
                || candidate.is_some()
            {
                return Err("owner measurement schema or identity".into());
            }
            let mut last = 0.;
            for b in m.spectrum {
                if ![b.frequency_hz, b.magnitude, b.phase_radians, b.coherence]
                    .iter()
                    .all(|v| v.is_finite())
                    || b.frequency_hz <= last
                    || b.frequency_hz < f64::from(options.band_hz[0])
                    || b.frequency_hz > f64::from(options.band_hz[1])
                    || b.magnitude < 0.
                    || b.phase_radians.abs() > std::f64::consts::PI
                    || !(0.0..=1.).contains(&b.coherence)
                {
                    return Err("owner spectrum schema".into());
                }
                last = b.frequency_hz;
            }
            Ok(("measured".into(), None, value, None))
        }
        Expected::Proposal {
            ids,
            meta,
            operation,
            target,
        } => {
            let expected_contract = match operation.as_str() {
                "eq_propose" => "C-PA-EQ-PROPOSAL",
                "overlap" => "C-PA-OVERLAP-REVIEW",
                "propose" => "C-PA-ALIGNMENT-PROPOSAL",
                _ => return Err("owner operation identity".into()),
            };
            if value["contract"] != expected_contract {
                return Err("owner result operation mismatch".into());
            }
            if value["contract"] == "C-PA-EQ-PROPOSAL" {
                if value["target"].as_str() != target.as_deref() {
                    return Err("EQ target mismatch".into());
                }
                return validate_eq_result(record, value, candidate, ids, meta);
            }
            let document = if value["contract"] == "C-PA-OVERLAP-REVIEW" {
                if value["version"] != 1 || value.as_object().is_none_or(|o| o.len() != 4) {
                    return Err("overlap envelope".into());
                }
                let predictions = value["predictions"]
                    .as_array()
                    .ok_or("overlap predictions")?;
                if predictions.len() > 8 {
                    return Err("overlap position bound".into());
                }
                for bins in predictions {
                    let bins: Vec<Bin> =
                        serde_json::from_value(bins.clone()).map_err(|e| e.to_string())?;
                    if bins.len() > 64
                        || bins.iter().any(|b| {
                            ![b.frequency_hz, b.magnitude, b.phase_radians, b.coherence]
                                .iter()
                                .all(|v| v.is_finite())
                                || !(20.0..=20000.0).contains(&b.frequency_hz)
                                || b.magnitude < 0.
                                || !(0.0..=1.0).contains(&b.coherence)
                                || b.phase_radians.abs() > std::f64::consts::PI
                        })
                    {
                        return Err("overlap prediction bounds".into());
                    }
                }
                &value["proposal"]
            } else {
                &value
            };
            let p: Proposal =
                serde_json::from_value(document.clone()).map_err(|e| e.to_string())?;
            let base: Value = measurement_owner::decode_document(
                record.basis.configuration_json.as_bytes(),
                48 * 1024,
            )?;
            if p.contract != "C-PA-ALIGNMENT-PROPOSAL"
                || p.version != 1
                || !matches!(p.status.as_str(), "proposed" | "no_change" | "refused")
                || p.basis_configuration != base
                || p.basis_configuration_revision != record.basis.graph_generation
                || &p.basis_capture != meta
                || !p.verification_required
                || p.reason.len() > 256
                || p.measurement_ids.len() > ids.len()
                || p.measurement_ids != ids[..p.measurement_ids.len()]
                || p.changes.len() > 2
                || p.added_latency_samples > 480
            {
                return Err("owner proposal schema or identity".into());
            }
            if p.status == "refused" {
                if !p.changes.is_empty() || p.added_latency_samples != 0 || candidate.is_some() {
                    return Err("refused proposal changes".into());
                }
                return Ok(("refused".into(), Some(p.reason), value, None));
            }
            if p.measurement_ids != *ids
                || (p.status == "no_change" && !p.changes.is_empty())
                || (p.status == "proposed" && p.changes.is_empty())
            {
                return Err("proposal state".into());
            }
            let c: Candidate = serde_json::from_value(candidate.ok_or("owner candidate missing")?)
                .map_err(|e| e.to_string())?;
            if c.contract != "C-PA-ALIGNMENT-CANDIDATE"
                || c.version != 1
                || c.basis_capture != *meta
                || c.basis_configuration_revision != record.basis.graph_generation
                || c.configuration_json.len() > 48 * 1024
            {
                return Err("owner candidate identity".into());
            }
            let actual: Value =
                measurement_owner::decode_document(c.configuration_json.as_bytes(), 48 * 1024)?;
            let mut checked = actual.clone();
            let mut seen = std::collections::BTreeSet::new();
            let mut added = 0;
            for change in &p.changes {
                if !seen.insert(change.output_index)
                    || change.after_delay_samples < change.before_delay_samples
                    || change.after_delay_samples > 480
                {
                    return Err("candidate change bounds".into());
                }
                let old = base["outputs"]
                    .as_array()
                    .and_then(|a| a.get(change.output_index))
                    .ok_or("candidate output")?;
                let new = checked["outputs"]
                    .as_array_mut()
                    .and_then(|a| a.get_mut(change.output_index))
                    .ok_or("candidate output")?;
                let before = old["processing"]["delay_ms"]
                    .as_f64()
                    .ok_or("candidate delay")?;
                let after = new["processing"]["delay_ms"]
                    .as_f64()
                    .ok_or("candidate delay")?;
                if (before * 48.).round() as u32 != change.before_delay_samples
                    || (after * 48.).round() as u32 != change.after_delay_samples
                    || old["processing"]["inverted"] != json!(change.before_inverted)
                    || new["processing"]["inverted"] != json!(change.after_inverted)
                {
                    return Err("candidate change mismatch".into());
                }
                added = added.max(change.after_delay_samples - change.before_delay_samples);
                new["processing"]["delay_ms"] = old["processing"]["delay_ms"].clone();
                new["processing"]["inverted"] = old["processing"]["inverted"].clone();
            }
            if checked != base || added != p.added_latency_samples {
                return Err("candidate changed protected graph state".into());
            }
            Ok((p.status, None, value, Some(c.configuration_json)))
        }
    }
}
fn validate_eq_result(
    record: &Record,
    value: Value,
    candidate: Option<Value>,
    ids: &[String],
    meta: &Meta,
) -> Result<(String, Option<String>, Value, Option<String>)> {
    let base: Value =
        measurement_owner::decode_document(record.basis.configuration_json.as_bytes(), 48 * 1024)?;
    let p = &value;
    let keys = [
        "contract",
        "version",
        "status",
        "reason",
        "basis_capture",
        "basis_configuration",
        "basis_configuration_revision",
        "measurement_ids",
        "output_index",
        "target",
        "calibration",
        "bands",
        "before_rms_db",
        "predicted_rms_db",
        "required_headroom_db",
        "verification_required",
    ];
    if p.as_object()
        .is_none_or(|o| o.len() != keys.len() || o.keys().any(|k| !keys.contains(&k.as_str())))
        || !p["calibration"].is_null()
        || !p["reason"].as_str().is_some_and(|s| s.len() <= 256)
        || ["before_rms_db", "predicted_rms_db", "required_headroom_db"]
            .iter()
            .any(|k| !p[k].as_f64().is_some_and(|v| v.is_finite() && v >= 0.))
    {
        return Err("EQ evidence schema".into());
    }
    if p["version"] != 1
        || p["basis_configuration"] != base
        || p["basis_configuration_revision"] != json!(record.basis.graph_generation)
        || p["basis_capture"] != json!(meta)
        || p["measurement_ids"] != json!(ids)
        || p["verification_required"] != true
    {
        return Err("EQ owner identity".into());
    }
    let status = p["status"].as_str().ok_or("EQ status")?.to_string();
    let bands = p["bands"].as_array().ok_or("EQ bands")?;
    if matches!(status.as_str(), "refused" | "no_change") {
        if !bands.is_empty() || candidate.is_some() {
            return Err("unchanged EQ candidate".into());
        }
        return Ok((status, p["reason"].as_str().map(String::from), value, None));
    }
    if status != "proposed" || bands.is_empty() || bands.len() > 8 {
        return Err("EQ proposal state".into());
    }
    let c: Candidate = serde_json::from_value(candidate.ok_or("EQ candidate missing")?)
        .map_err(|e| e.to_string())?;
    if c.contract != "C-PA-EQ-CANDIDATE"
        || c.version != 1
        || c.basis_capture != *meta
        || c.basis_configuration_revision != record.basis.graph_generation
    {
        return Err("EQ candidate identity".into());
    }
    let actual: Value =
        measurement_owner::decode_document(c.configuration_json.as_bytes(), 48 * 1024)?;
    let mut expected = base.clone();
    let index = p["output_index"].as_u64().ok_or("EQ output")? as usize;
    if index != meta.output_index {
        return Err("EQ output identity".into());
    }
    let output = expected["outputs"]
        .as_array_mut()
        .and_then(|v| v.get_mut(index))
        .ok_or("EQ output")?;
    if output["processing"]["eq_enabled"] != true || output["source"].is_null() {
        return Err("EQ bank unavailable".into());
    }
    let slots = output["processing"]["eq"]
        .as_array_mut()
        .ok_or("EQ slots")?;
    let mut band_iter = bands.iter();
    let mut boost = 0.;
    let mut cut = 0.;
    for slot in slots {
        if slot["db"].as_f64() == Some(0.)
            && let Some(b) = band_iter.next()
        {
            if b.as_object().is_none_or(|o| o.len() != 5)
                || b["kind"] != "bell"
                || b["slope"].as_f64() != Some(1.)
            {
                return Err("EQ band schema".into());
            }
            let hz = b["hz"].as_f64().ok_or("EQ frequency")?;
            let q = b["q"].as_f64().ok_or("EQ Q")?;
            let db = b["db"].as_f64().ok_or("EQ gain")?;
            if !(50.0..=12000.0).contains(&hz)
                || !(0.5..=2.0).contains(&q)
                || !(-6.0..=3.0).contains(&db)
            {
                return Err("EQ band bounds".into());
            }
            boost += db.max(0.);
            cut += db.min(0.);
            *slot = b.clone();
        }
    }
    if band_iter.next().is_some()
        || boost > 3.
        || cut < -12.
        || p["required_headroom_db"].as_f64() != Some(boost)
        || actual != expected
    {
        return Err("EQ changed protected state or limits".into());
    }
    Ok((status, None, value, Some(c.configuration_json)))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> State {
        use sha2::{Digest, Sha256};
        let path = std::fs::canonicalize("/usr/bin/true").unwrap();
        let hash = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
        State::new(measurement_owner::Config {
            version: 1,
            mode: "software-only".into(),
            owner_executable: path,
            owner_sha256: hash,
        })
        .unwrap()
    }
    fn basis() -> Basis {
        Basis {
            source_epoch: Counter(1),
            map_revision: Counter(1),
            graph_generation: Counter(1),
            clock_domain: "software-common-source".into(),
            configuration_json: r#"{"outputs":[{"source":{"input":0}}]}"#.into(),
            program_buses: vec![0, 1],
        }
    }
    fn topology() -> crate::topology::EngineTopology {
        let mut t = crate::topology::EngineTopology::software(16, 2, 0).unwrap();
        t.capture_channels += 1;
        t.measurement_slots = vec![16];
        t.validate(crate::topology::ResourceBudget::default())
            .unwrap();
        t
    }
    fn auth() -> crate::control_model::Request {
        crate::control_model::Request {
            contract: "C-AUDIO".into(),
            version: 2,
            show_id: "11111111-1111-4111-8111-111111111111".into(),
            module: "audio".into(),
            epoch: Counter(1),
            writer: Some("test".into()),
            lease: Some(Counter(1)),
            request_id: Some(Counter(2)),
            expected_revision: Some(Counter(0)),
            command: crate::control_model::Command::Renew {},
        }
    }
    fn start() -> Command {
        Command::CaptureStart {
            id: "take-a".into(),
            reference_input: "input-03".into(),
            mic_capture_slot: 16,
            output_index: 0,
            position_id: "p1".into(),
            samples: 32768,
            options: Options::default(),
        }
    }
    #[test]
    fn capture_exact_reference_and_reserved_mic_no_buffer_growth() {
        let mut s = state();
        let t = topology();
        let a = s.prepare(&start(), &basis(), &t, 48, 0).unwrap();
        s.apply(a, auth(), basis()).unwrap();
        let before = (
            s.capture.as_ref().unwrap().reference.as_ptr(),
            s.capture.as_ref().unwrap().mic.as_ptr(),
        );
        let mut block = vec![0.; 48 * 17];
        for f in 0..48 {
            block[f * 17 + 2] = f as f64 / 100.;
            block[f * 17 + 16] = -(f as f64) / 100.;
        }
        s.offer(1, 1, 48, &block, true);
        let c = s.capture.as_ref().unwrap();
        assert_eq!(c.reference.len(), 48);
        for i in 0..48 {
            assert_eq!(c.reference[i], block[i * 17 + 2]);
            assert_eq!(c.mic[i], block[i * 17 + 16]);
        }
        assert_eq!(before, (c.reference.as_ptr(), c.mic.as_ptr()));
        assert!(t.inputs.iter().all(|p| p.capture_slot != 16));
    }
    #[test]
    fn program_mic_and_shape_clock_clipping_cancellation_refuse() {
        let mut s = state();
        let t = topology();
        let mut command = start();
        if let Command::CaptureStart {
            mic_capture_slot, ..
        } = &mut command
        {
            *mic_capture_slot = 0;
        }
        assert!(s.prepare(&command, &basis(), &t, 0, 0).is_err());
        for failure in 0..5 {
            let mut s = state();
            let a = s.prepare(&start(), &basis(), &t, 0, 0).unwrap();
            s.apply(a, auth(), basis()).unwrap();
            let mut block = vec![0.; 48 * 17];
            if failure == 2 {
                block[16] = 1.;
            }
            s.offer(
                if failure == 0 { 2 } else { 1 },
                if failure == 4 { 2 } else { 1 },
                if failure == 1 { 48 } else { 0 },
                &block,
                failure != 3,
            );
            s.poll(1);
            assert_eq!(s.records[0].summary.state, "invalidated");
            assert!(!s.busy());
        }
        let a = s.prepare(&start(), &basis(), &t, 0, 0).unwrap();
        s.apply(a, auth(), basis()).unwrap();
        s.invalidate("take-a", "cancelled");
        assert_eq!(s.records[0].summary.state, "cancelled");
        assert!(s.records[0].summary.reason.is_some());
    }
    #[test]
    fn capture_limits_and_deadline_are_bounded() {
        let mut s = state();
        let t = topology();
        let mut command = start();
        if let Command::CaptureStart { samples, .. } = &mut command {
            *samples = usize::MAX;
        }
        assert!(s.prepare(&command, &basis(), &t, 0, 0).is_err());
        let a = s.prepare(&start(), &basis(), &t, 0, 0).unwrap();
        s.apply(a, auth(), basis()).unwrap();
        s.poll(5001);
        assert_eq!(
            s.records[0].summary.reason.as_deref(),
            Some("capture deadline")
        );
    }
    #[test]
    fn retained_measurements_require_the_original_writer_lease() {
        let mut s = state();
        let a = s.prepare(&start(), &basis(), &topology(), 0, 0).unwrap();
        s.apply(a, auth(), basis()).unwrap();
        let cancel = Command::CaptureCancel {
            id: "take-a".into(),
        };
        assert!(s.authorize_records(&cancel, &auth()).is_ok());
        let mut other = auth();
        other.writer = Some("other".into());
        assert!(s.authorize_records(&cancel, &other).is_err());
        other = auth();
        other.lease = Some(Counter(2));
        assert!(s.authorize_records(&cancel, &other).is_err());
        let propose = Command::MeasurementPropose {
            id: "proposal".into(),
            pairs: vec![["take-a".into(), "take-a".into()]],
        };
        assert!(s.authorize_records(&propose, &other).is_err());
    }
    #[test]
    fn owner_measurement_metadata_and_schema_are_fenced() {
        let mut s = state();
        let a = s.prepare(&start(), &basis(), &topology(), 0, 0).unwrap();
        s.apply(a, auth(), basis()).unwrap();
        let meta = s.capture.as_ref().unwrap().meta.clone();
        let options = Options::default();
        let expected = Expected::Measurement {
            meta: meta.clone(),
            options: options.clone(),
            samples: 32768,
        };
        let good = json!({"contract":"C-PA-MEASUREMENT-RESULT","version":1,"algorithm":"offline-h1-v1","capture":meta,"options":options,"samples":32768,"arrival_samples":0,"signed_correlation":0.1,"competing_peak_ratio":0.8,"segments":15,"spectrum":[{"frequency_hz":1000.0,"magnitude":1.0,"phase_radians":0.0,"coherence":0.2}],"proposal_eligible":false,"reasons":["weak_coherence"]});
        assert!(validate_result(&expected, &s.records[0], good.clone(), None).is_ok());
        for (path, bad) in [
            ("segments", json!(8)),
            ("spectrum", json!([])),
            ("samples", json!(65536)),
            ("proposal_eligible", json!(true)),
        ] {
            let mut v = good.clone();
            v[path] = bad;
            assert!(validate_result(&expected, &s.records[0], v, None).is_err());
        }
        for key in [
            "first_frame",
            "map_revision",
            "configuration_revision",
            "source_epoch",
        ] {
            let mut v = good.clone();
            v["capture"][key] = json!("99");
            assert!(validate_result(&expected, &s.records[0], v, None).is_err());
        }
        let mut v = good;
        v["spectrum"][0]["coherence"] = json!(1.1);
        assert!(validate_result(&expected, &s.records[0], v, None).is_err());
    }
    #[test]
    fn revoked_pending_capture_retains_named_terminal_state() {
        let mut s = state();
        let prepared = s.prepare(&start(), &basis(), &topology(), 48, 0).unwrap();
        s.invalidate_prepared(prepared, auth(), basis(), "measurement writer revoked");
        assert_eq!(s.records[0].summary.id, "take-a");
        assert_eq!(s.records[0].summary.state, "invalidated");
        assert_eq!(
            s.records[0].summary.reason.as_deref(),
            Some("measurement writer revoked")
        );
        assert!(!s.busy());
        assert_eq!(s.guards().count(), 0);
    }
}
