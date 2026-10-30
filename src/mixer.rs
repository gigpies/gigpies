//! Configurable preallocated mono-strip graph. The fixed arrays are legacy adapters.
use crate::control_model::{Edit, Target, Value};
use crate::show::Result;

pub const INPUTS: usize = 8;
pub const RAMP_FRAMES: u64 = 240;
pub const BOUNDARY_FRAMES: u64 = 48;
#[derive(Debug, Clone, Copy)]
struct Ramp {
    start: f64,
    target: f64,
    begin: u64,
    end: u64,
}
impl Ramp {
    fn fixed(value: f64) -> Self {
        Self {
            start: value,
            target: value,
            begin: 0,
            end: 0,
        }
    }
    fn at(self, frame: u64) -> f64 {
        if frame >= self.end {
            self.target
        } else if frame <= self.begin {
            self.start
        } else {
            self.start
                + (self.target - self.start)
                    * ((frame - self.begin) as f64 / (self.end - self.begin) as f64)
        }
    }
}
#[derive(Debug, Clone, Copy, Default)]
struct TapTransition {
    current: crate::sends_wire::Tap,
    target: crate::sends_wire::Tap,
    begin: u64,
    end: u64,
}
#[derive(Debug, Clone, Copy)]
enum Change {
    Keep,
    Ramp(f64),
    Freeze,
}
/// All transcendental arithmetic and target/string validation occurs here, outside process.
#[derive(Debug, Clone)]
pub struct Prepared {
    changes: Vec<Vec<Change>>,
    processing: Option<(usize, crate::channel_processing::Prepared)>,
    send: Option<(usize, usize, crate::sends_wire::Tap)>,
}
impl Prepared {
    pub fn processing(input: usize, prepared: crate::channel_processing::Prepared) -> Result<Self> {
        Self::processing_for(INPUTS, 2, input, prepared)
    }
    pub fn processing_for(
        inputs: usize,
        monitors: usize,
        input: usize,
        prepared: crate::channel_processing::Prepared,
    ) -> Result<Self> {
        if input >= inputs {
            return Err("target".into());
        }
        Ok(Self {
            changes: vec![vec![Change::Keep; 4 + monitors]; inputs],
            processing: Some((input, prepared)),
            send: None,
        })
    }
    pub fn send_for(
        inputs: usize,
        monitors: usize,
        input: usize,
        monitor: usize,
        tap: crate::sends_wire::Tap,
    ) -> Result<Self> {
        if input >= inputs || monitor >= monitors {
            return Err("target".into());
        }
        Ok(Self {
            changes: vec![vec![Change::Keep; 4 + monitors]; inputs],
            processing: None,
            send: Some((input, monitor, tap)),
        })
    }
    pub fn edits(edits: &[Edit]) -> Result<Self> {
        Self::edits_for(INPUTS, 2, edits)
    }
    pub fn edits_for(inputs: usize, monitors: usize, edits: &[Edit]) -> Result<Self> {
        if edits.is_empty() || edits.len() > 64 {
            return Err("capacity".into());
        }
        let mut p = Self {
            changes: vec![vec![Change::Keep; 4 + monitors]; inputs],
            processing: None,
            send: None,
        };
        let mut seen = vec![vec![false; 3 + monitors]; inputs];
        for e in edits {
            let (input, slot) = target_slot(&e.target, inputs, monitors)?;
            if seen[input][slot] {
                return Err("duplicate target".into());
            }
            seen[input][slot] = true;
            match (&e.target, &e.value) {
                (Target::Fader { .. }, Value::Integer(n))
                | (Target::Send { .. }, Value::Integer(n))
                    if (-60000..=12000).contains(n) && n % 100 == 0 =>
                {
                    let index = if slot == 0 { 0 } else { slot + 1 };
                    p.changes[input][index] = Change::Ramp(10_f64.powf(*n as f64 / 20000.0));
                }
                (Target::Pan { .. }, Value::Integer(n)) if (-100..=100).contains(n) => {
                    let angle = (*n as f64 + 100.0) * std::f64::consts::FRAC_PI_2 / 200.0;
                    let (l, r) = if *n == -100 {
                        (1.0, 0.0)
                    } else if *n == 100 {
                        (0.0, 1.0)
                    } else {
                        (angle.cos(), angle.sin())
                    };
                    p.changes[input][1] = Change::Ramp(l);
                    p.changes[input][2] = Change::Ramp(r);
                }
                (Target::Mute { .. }, Value::Boolean(mute)) => {
                    p.changes[input][3] = Change::Ramp(if *mute { 0.0 } else { 1.0 })
                }
                _ => return Err("range".into()),
            }
        }
        Ok(p)
    }
    pub fn freeze(scope: crate::control_model::Scope) -> Self {
        Self::freeze_for(INPUTS, 2, scope)
    }
    pub fn freeze_for(inputs: usize, monitors: usize, scope: crate::control_model::Scope) -> Self {
        let mut p = Self {
            changes: vec![vec![Change::Keep; 4 + monitors]; inputs],
            processing: None,
            send: None,
        };
        for row in &mut p.changes {
            match scope {
                crate::control_model::Scope::Foh => row[..4].fill(Change::Freeze),
                crate::control_model::Scope::Monitor1 if monitors >= 1 => row[4] = Change::Freeze,
                crate::control_model::Scope::Monitor2 if monitors >= 2 => row[5] = Change::Freeze,
                crate::control_model::Scope::Monitor(n) if usize::from(n) <= monitors && n > 0 => {
                    row[usize::from(n) + 3] = Change::Freeze
                }
                _ => (),
            }
        }
        p
    }
}
fn target_slot(target: &Target, inputs: usize, monitors: usize) -> Result<(usize, usize)> {
    let (input, slot) = match target {
        Target::Fader { input } => (input, 0),
        Target::Pan { input } => (input, 1),
        Target::Mute { input } => (input, 2),
        Target::Send { input, monitor } => {
            let n = monitor
                .strip_prefix("monitor-")
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|&n| n > 0 && n <= monitors)
                .ok_or("target")?;
            if monitor != &format!("monitor-{n}") {
                return Err("target".into());
            }
            (input, 2 + n)
        }
    };
    let index = crate::processing_wire::input_index(input)?;
    if index >= inputs {
        return Err("target".into());
    }
    Ok((index, slot))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Completion {
    pub ticket: u64,
    pub frame: u64,
}
#[derive(Debug, Clone)]
struct Pending {
    prepared: Prepared,
    frame: u64,
    ticket: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    BufferLength,
    ClockExhausted,
    Faulted,
}
/// Owns the candidate until commit, then the retired topology until controller
/// destruction. Applying never allocates, validates strings, or destroys ownership.
#[derive(Debug)]
pub struct PreparedOutputPatch {
    topology: crate::topology::EngineTopology,
    base: crate::topology::EngineTopology,
    applied: bool,
}
#[derive(Debug)]
pub struct Mixer {
    meter_processed: Vec<f64>,
    meter_valid: [bool; 48],
    operator_source: crate::brain_control::MonitorSource,
    operator_samples: [f64; 96],
    clock: u64,
    ramps: Vec<Vec<Ramp>>,
    sends: Vec<Vec<TapTransition>>,
    fx_sends: Vec<[Ramp; 4]>,
    fx_samples: [[f64; 96]; 4],
    pending: Option<Pending>,
    completion: Option<Completion>,
    fault: bool,
    processing: Vec<crate::channel_processing::Strip>,
    retired: Option<Pending>,
    armed: bool,
    output_gain: Ramp,
    topology: crate::topology::EngineTopology,
}
impl Default for Mixer {
    fn default() -> Self {
        Self::new(0)
    }
}
impl Mixer {
    pub fn new(frame: u64) -> Self {
        Self::from_topology(frame, crate::topology::EngineTopology::legacy())
            .expect("legacy topology")
    }
    pub fn from_topology(frame: u64, topology: crate::topology::EngineTopology) -> Result<Self> {
        topology.validate(crate::topology::ResourceBudget::default())?;
        let inputs = topology.inputs.len();
        let monitors = topology.monitors;
        let gain = 10_f64.powf(-6.0 / 20.0);
        let mut row = vec![
            Ramp::fixed(gain),
            Ramp::fixed(std::f64::consts::FRAC_1_SQRT_2),
            Ramp::fixed(std::f64::consts::FRAC_1_SQRT_2),
            Ramp::fixed(1.0),
        ];
        row.extend((0..monitors).map(|i| Ramp::fixed(if i == 0 { 1.0 } else { 0.001 })));
        Ok(Self {
            meter_processed: if topology
                .meter_admission(crate::topology::ResourceBudget::default())
                .is_ok()
            {
                vec![0.; inputs * 48]
            } else {
                Vec::new()
            },
            meter_valid: [false; 48],
            operator_source: crate::brain_control::MonitorSource::None,
            operator_samples: [0.; 96],
            clock: frame,
            ramps: vec![row; inputs],
            sends: vec![vec![TapTransition::default(); monitors]; inputs],
            fx_sends: Vec::new(),
            fx_samples: [[0.; 96]; 4],
            pending: None,
            completion: None,
            fault: false,
            processing: vec![crate::channel_processing::Strip::default(); inputs],
            retired: None,
            topology,
            armed: true,
            output_gain: Ramp::fixed(1.),
        })
    }
    /// Optional aux provisioning is control-side. Preserve the published core
    /// resource/snapshot contract; admit this extension against the remaining budget.
    pub fn enable_fx_aux(&mut self) -> Result<()> {
        if !self.fx_sends.is_empty() {
            return Ok(());
        }
        let budget = crate::topology::ResourceBudget::default();
        let core = self.topology.validate(budget)?;
        let bytes = self
            .input_count()
            .checked_mul(4 * size_of::<Ramp>())
            .and_then(|n| n.checked_add(size_of::<[[f64; 96]; 4]>()))
            .and_then(|n| n.checked_add(core.estimated_bytes))
            .ok_or("FX aux memory overflow")?;
        let work = self
            .input_count()
            .checked_mul(8)
            .and_then(|n| n.checked_mul(self.topology.max_block_frames))
            .and_then(|n| n.checked_add(core.sample_operations))
            .ok_or("FX aux work overflow")?;
        if bytes > budget.bytes || work > budget.sample_operations {
            return Err("FX aux resource admission".into());
        }
        self.fx_sends = vec![[Ramp::fixed(0.); 4]; self.input_count()];
        Ok(())
    }
    pub(crate) fn inherit_fx_send_intent(&mut self, previous: &Self) -> Result<()> {
        if previous.fx_sends.is_empty() {
            return Ok(());
        }
        if self.input_count() != previous.input_count() {
            return Err("FX aux recovery dimensions".into());
        }
        self.enable_fx_aux()?;
        for (new, old) in self.fx_sends.iter_mut().zip(&previous.fx_sends) {
            for (new, old) in new.iter_mut().zip(old) {
                *new = Ramp::fixed(old.target);
            }
        }
        Ok(())
    }
    /// Control-side validated, source-boundary application. No allocation in render.
    pub fn set_fx_sends(&mut self, bus: usize, levels: &[f64]) -> Result<()> {
        if bus >= 4
            || self.fx_sends.len() != self.input_count()
            || levels.len() != self.input_count()
            || levels
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || self.clock.checked_add(RAMP_FRAMES).is_none()
        {
            return Err("FX aux send shape/range".into());
        }
        for (row, &target) in self.fx_sends.iter_mut().zip(levels) {
            row[bus] = Ramp {
                start: row[bus].at(self.clock),
                target,
                begin: self.clock,
                end: self.clock + RAMP_FRAMES,
            };
        }
        Ok(())
    }
    pub fn fx_sends(&self, bus: usize) -> Option<Vec<f64>> {
        (bus < 4 && !self.fx_sends.is_empty())
            .then(|| self.fx_sends.iter().map(|r| r[bus].target).collect())
    }
    pub fn fx_aux_samples(&self) -> &[[f64; 96]; 4] {
        &self.fx_samples
    }
    pub fn set_operator_tap(&mut self, source: crate::brain_control::MonitorSource) {
        self.operator_source = source;
        self.operator_samples.fill(0.);
    }
    pub fn operator_tap(&self) -> &[f64; 96] {
        &self.operator_samples
    }
    pub fn output_safety_gain(&self, frame: u64) -> f64 {
        if self.fault || !self.armed {
            0.
        } else {
            self.output_gain.at(frame)
        }
    }
    pub fn restore_intent(
        &mut self,
        edits: &[Edit],
        configs: &[crate::channel_processing::Config],
    ) -> Result<()> {
        if configs.len() != self.input_count() {
            return Err("intent processing shape".into());
        }
        // Prepare all candidates before mutation; full intent may exceed the wire
        // transaction's 64-target bound, so provision one validated strip at a time.
        let mut changes = vec![vec![Change::Keep; 4 + self.monitor_count()]; self.input_count()];
        let mut seen = std::collections::BTreeSet::new();
        for edit in edits {
            if !seen.insert(edit.target.clone()) {
                return Err("duplicate intent".into());
            }
            let p = Prepared::edits_for(
                self.input_count(),
                self.monitor_count(),
                std::slice::from_ref(edit),
            )?;
            for (row, new) in changes.iter_mut().zip(p.changes) {
                for (old, new) in row.iter_mut().zip(new) {
                    if !matches!(new, Change::Keep) {
                        *old = new;
                    }
                }
            }
        }
        let prepared: Vec<_> = configs
            .iter()
            .map(|&c| crate::channel_processing::Prepared::new(c))
            .collect::<Result<_>>()?;
        for (row, new) in self.ramps.iter_mut().zip(changes) {
            for (old, new) in row.iter_mut().zip(new) {
                if let Change::Ramp(v) = new {
                    *old = Ramp::fixed(v);
                }
            }
        }
        for (strip, prepared) in self.processing.iter_mut().zip(prepared) {
            strip.apply(prepared, self.clock);
        }
        self.quiesce();
        Ok(())
    }
    pub fn quiesce(&mut self) {
        self.armed = false;
        self.output_gain = Ramp::fixed(0.);
        if self.pending.is_some() {
            debug_assert!(self.retired.is_none());
            self.retired = self.pending.take();
        }
        self.completion = None;
    }
    pub fn rearm(&mut self) -> std::result::Result<(), ProcessError> {
        let end = self
            .clock
            .checked_add(RAMP_FRAMES)
            .ok_or(ProcessError::ClockExhausted)?;
        if self.fault {
            return Err(ProcessError::Faulted);
        }
        self.armed = true;
        self.output_gain = Ramp {
            start: 0.,
            target: 1.,
            begin: self.clock,
            end,
        };
        Ok(())
    }
    pub fn mute_outputs(&mut self) -> std::result::Result<(), ProcessError> {
        let end = self
            .clock
            .checked_add(RAMP_FRAMES)
            .ok_or(ProcessError::ClockExhausted)?;
        self.output_gain = Ramp {
            start: self.output_gain.at(self.clock),
            target: 0.,
            begin: self.clock,
            end,
        };
        Ok(())
    }
    pub fn outputs_quiesced(&self) -> bool {
        !self.armed || (self.output_gain.target == 0. && self.clock >= self.output_gain.end)
    }
    pub fn prepare_output_patch(
        &self,
        outputs: Vec<crate::topology::OutputPort>,
    ) -> Result<PreparedOutputPatch> {
        let mut topology = self.topology.clone();
        topology.outputs = outputs;
        topology.map_revision = topology
            .map_revision
            .checked_add(1)
            .ok_or("map exhausted")?;
        topology.validate(crate::topology::ResourceBudget::default())?;
        Ok(PreparedOutputPatch {
            topology,
            base: self.topology.clone(),
            applied: false,
        })
    }
    pub fn apply_output_patch(
        &mut self,
        prepared: &mut PreparedOutputPatch,
    ) -> std::result::Result<(), &'static str> {
        if !self.outputs_quiesced() || prepared.applied || self.topology != prepared.base {
            return Err("patch quiescence/identity");
        }
        std::mem::swap(&mut self.topology, &mut prepared.topology);
        prepared.applied = true;
        Ok(())
    }
    /// Control-side convenience only; production boundaries use prepare/apply.
    pub fn replace_output_patch(
        &mut self,
        outputs: Vec<crate::topology::OutputPort>,
    ) -> Result<()> {
        let mut prepared = self.prepare_output_patch(outputs)?;
        self.apply_output_patch(&mut prepared).map_err(String::from)
    }
    pub fn topology(&self) -> &crate::topology::EngineTopology {
        &self.topology
    }
    pub fn input_count(&self) -> usize {
        self.processing.len()
    }
    pub fn monitor_count(&self) -> usize {
        self.topology.monitors
    }
    pub fn frame(&self) -> u64 {
        self.clock
    }
    pub fn faulted(&self) -> bool {
        self.fault
    }
    pub fn sends_ready(&self) -> bool {
        self.sends.iter().flatten().all(|s| self.clock >= s.end)
    }
    pub fn send_observations(
        &self,
    ) -> Vec<Vec<(crate::sends_wire::Tap, crate::sends_wire::Tap, u32)>> {
        self.sends
            .iter()
            .map(|row| {
                row.iter()
                    .map(|s| {
                        let remaining = s.end.saturating_sub(self.clock) as u32;
                        (
                            if remaining == 0 { s.target } else { s.current },
                            s.target,
                            remaining,
                        )
                    })
                    .collect()
            })
            .collect()
    }
    pub fn restore_sends(&mut self, taps: &[Vec<crate::sends_wire::Tap>]) -> Result<()> {
        if taps.len() != self.input_count() || taps.iter().any(|r| r.len() != self.monitor_count())
        {
            return Err("intent sends shape".into());
        }
        for (row, taps) in self.sends.iter_mut().zip(taps) {
            for (s, &tap) in row.iter_mut().zip(taps) {
                *s = TapTransition {
                    current: tap,
                    target: tap,
                    begin: 0,
                    end: 0,
                };
            }
        }
        Ok(())
    }
    pub fn processing_observations(
        &self,
    ) -> Vec<(
        crate::channel_processing::Config,
        crate::channel_processing::Config,
        u32,
        Option<i32>,
    )> {
        self.processing
            .iter()
            .map(|s| s.observation(self.clock, self.fault))
            .collect()
    }
    pub fn meter_samples(&self) -> (&[f64], &[bool; 48]) {
        (&self.meter_processed, &self.meter_valid)
    }
    pub fn processing_ready(&self) -> bool {
        self.processing
            .iter()
            .all(|s| s.observation(self.clock, self.fault).2 == 0)
    }
    pub fn next_boundary(&self) -> Result<u64> {
        self.clock
            .checked_div(48)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| n.checked_mul(48))
            .ok_or("clock exhausted".into())
    }
    pub fn schedule(&mut self, prepared: Prepared, ticket: u64) -> Result<u64> {
        if prepared.changes.len() != self.input_count()
            || prepared
                .changes
                .iter()
                .any(|r| r.len() != 4 + self.monitor_count())
        {
            return Err("prepared shape".into());
        }
        if self.pending.is_some()
            || self.retired.is_some()
            || self.completion.is_some()
            || (prepared.processing.is_some() && !self.processing_ready())
            || (prepared.send.is_some() && !self.sends_ready())
        {
            return Err("capacity".into());
        }
        let frame = self.next_boundary()?;
        frame.checked_add(RAMP_FRAMES).ok_or("clock exhausted")?;
        if ticket == 0 {
            return Err("ticket".into());
        }
        self.pending = Some(Pending {
            prepared,
            frame,
            ticket,
        });
        Ok(frame)
    }
    pub fn cancel(&mut self, ticket: u64) -> bool {
        if self.pending.as_ref().is_some_and(|p| p.ticket == ticket) {
            self.pending = None;
            true
        } else {
            false
        }
    }
    pub fn take_completion(&mut self) -> Option<Completion> {
        self.retired.take(); // control-side destruction, never inside render
        self.completion.take()
    }
    pub fn coefficients(&self) -> Vec<Vec<f64>> {
        self.ramps
            .iter()
            .map(|row| row.iter().map(|r| r.at(self.clock)).collect())
            .collect()
    }
    pub fn targets(&self) -> Vec<Vec<f64>> {
        self.ramps
            .iter()
            .map(|row| row.iter().map(|r| r.target).collect())
            .collect()
    }
    /// Frames are consumed in order. Boundary applies before its sample; the first
    /// sample uses the actual old coefficient, endpoint sample uses exact target.
    /// All storage is fixed and Copy; no heap values enter or leave this function.
    pub fn process(
        &mut self,
        input: &[[f64; INPUTS]],
        output: &mut [[f64; 4]],
    ) -> std::result::Result<(), ProcessError> {
        if !self.topology.is_legacy() {
            return Err(ProcessError::BufferLength);
        }
        self.process_interleaved(input.as_flattened(), output.as_flattened_mut())
    }
    /// No allocation, destruction, coefficient design, locks or I/O. Pending owned
    /// storage moves to a retirement slot and is destroyed by take_completion.
    pub fn process_interleaved(
        &mut self,
        input: &[f64],
        output: &mut [f64],
    ) -> std::result::Result<(), ProcessError> {
        let inputs = self.input_count();
        let outputs = self.monitor_count() + 2;
        if !input.len().is_multiple_of(inputs) || output.len() != input.len() / inputs * outputs {
            return Err(ProcessError::BufferLength);
        }
        let frames = input.len() / inputs;
        if !self.topology.is_legacy() && frames > self.topology.max_block_frames {
            return Err(ProcessError::BufferLength);
        }
        if self.clock.checked_add(frames as u64).is_none() {
            self.fault = true;
            output.fill(0.);
            return Err(ProcessError::ClockExhausted);
        }
        for (sources, out) in input
            .chunks_exact(inputs)
            .zip(output.chunks_exact_mut(outputs))
        {
            if let Some(p) = self.pending.as_ref()
                && p.frame == self.clock
            {
                for (row, changes) in self.ramps.iter_mut().zip(&p.prepared.changes) {
                    for (r, c) in row.iter_mut().zip(changes) {
                        let current = r.at(self.clock);
                        match *c {
                            Change::Keep => (),
                            Change::Freeze => *r = Ramp::fixed(current),
                            Change::Ramp(target) => {
                                *r = Ramp {
                                    start: current,
                                    target,
                                    begin: self.clock,
                                    end: self.clock + RAMP_FRAMES,
                                }
                            }
                        }
                    }
                }
                if let Some((input, prepared)) = p.prepared.processing {
                    self.processing[input].apply(prepared, self.clock);
                }
                if let Some((input, monitor, target)) = p.prepared.send {
                    let old = self.sends[input][monitor].target;
                    self.sends[input][monitor] = TapTransition {
                        current: old,
                        target,
                        begin: self.clock,
                        end: if old == target {
                            self.clock
                        } else {
                            self.clock + RAMP_FRAMES
                        },
                    };
                }
                self.completion = Some(Completion {
                    ticket: p.ticket,
                    frame: p.frame,
                });
                self.retired = self.pending.take();
            }
            out.fill(0.);
            let tap = (self.clock % 48) as usize * 2;
            self.operator_samples[tap..tap + 2].fill(0.);
            for aux in &mut self.fx_samples {
                aux[tap..tap + 2].fill(0.);
            }
            let meter_frame = (self.clock % 48) as usize;
            if !self.meter_processed.is_empty() {
                self.meter_processed[meter_frame * inputs..(meter_frame + 1) * inputs].fill(0.);
            }
            if sources.iter().any(|s| !s.is_finite()) {
                self.fault = true;
            }
            if !self.fault {
                for (channel, ((sample, row), strip)) in sources
                    .iter()
                    .zip(&self.ramps)
                    .zip(&mut self.processing)
                    .enumerate()
                {
                    if row.iter().any(|r| !r.at(self.clock).is_finite()) {
                        self.fault = true;
                        break;
                    }
                    let shared = sample * row[3].at(self.clock);
                    let processed = strip.tick(*sample, self.clock);
                    if !self.meter_processed.is_empty() {
                        self.meter_processed[meter_frame * inputs + channel] = processed;
                    }
                    if !processed.is_finite() {
                        self.fault = true;
                        break;
                    }
                    let foh = processed * row[3].at(self.clock);
                    use crate::brain_control::MonitorSource;
                    match self.operator_source {
                        MonitorSource::Pfl { input } if input == channel => {
                            self.operator_samples[tap] =
                                processed * std::f64::consts::FRAC_1_SQRT_2;
                            self.operator_samples[tap + 1] = self.operator_samples[tap];
                        }
                        MonitorSource::Afl { input } if input == channel => {
                            self.operator_samples[tap] =
                                foh * row[0].at(self.clock) * row[1].at(self.clock);
                            self.operator_samples[tap + 1] =
                                foh * row[0].at(self.clock) * row[2].at(self.clock);
                        }
                        _ => (),
                    }
                    out[0] += foh * row[0].at(self.clock) * row[1].at(self.clock);
                    out[1] += foh * row[0].at(self.clock) * row[2].at(self.clock);
                    if let Some(sends) = self.fx_sends.get(channel) {
                        for (aux, send) in self.fx_samples.iter_mut().zip(sends) {
                            let sent = foh * row[0].at(self.clock) * send.at(self.clock);
                            aux[tap] += sent * row[1].at(self.clock);
                            aux[tap + 1] += sent * row[2].at(self.clock);
                        }
                    }
                    for (monitor, (out, send)) in out[2..].iter_mut().zip(&row[4..]).enumerate() {
                        let mode = self.sends[channel][monitor];
                        let select = |tap| match tap {
                            crate::sends_wire::Tap::RawPostMute => shared,
                            crate::sends_wire::Tap::ProcessedPreFader => foh,
                            crate::sends_wire::Tap::ProcessedPostFader => {
                                foh * row[0].at(self.clock)
                            }
                        };
                        let signal = if self.clock >= mode.end {
                            select(mode.target)
                        } else if self.clock <= mode.begin {
                            select(mode.current)
                        } else {
                            let w = (self.clock - mode.begin) as f64 / RAMP_FRAMES as f64;
                            select(mode.current) * (1. - w) + select(mode.target) * w
                        };
                        *out += signal * send.at(self.clock);
                    }
                }
                if out.iter().any(|s| !s.is_finite()) {
                    self.fault = true;
                }
            }
            if self.fault {
                self.operator_samples[tap..tap + 2].fill(0.);
            }
            if self.fault || !self.armed {
                out.fill(0.);
            } else {
                let gain = self.output_gain.at(self.clock);
                if gain != 1. {
                    for v in out {
                        *v *= gain;
                    }
                }
            }
            let gain = self.output_safety_gain(self.clock);
            for aux in &mut self.fx_samples {
                for sample in &mut aux[tap..tap + 2] {
                    *sample = if gain == 0. || !sample.is_finite() {
                        0.
                    } else {
                        *sample * gain
                    };
                }
            }
            self.meter_valid[meter_frame] = !self.fault && self.armed;
            self.clock += 1;
        }
        Ok(())
    }
}
#[cfg(test)]
mod send_reservation_tests {
    use super::*;
    #[test]
    fn tap_storage_fits_existing_coefficient_reservation() {
        // Live ramp + tap state + admitted and retiring change matrices.
        // Every send is already included in the existing coefficient count.
        let bytes = std::mem::size_of::<Ramp>()
            + std::mem::size_of::<TapTransition>()
            + 2 * std::mem::size_of::<Change>();
        assert!(
            bytes <= 128,
            "per-send resident and prepared storage: {bytes}"
        );
        eprintln!("per-send scalar storage {bytes} bytes within prior 128-byte reservation");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupted_coefficient_latches_finite_zero_output() {
        let mut mixer = Mixer::default();
        mixer.ramps[3][4] = Ramp::fixed(f64::INFINITY);
        let mut output = [[1.0; 4]; 2];
        mixer.process(&[[1.0; INPUTS]; 2], &mut output).unwrap();
        assert_eq!(output, [[0.0; 4]; 2]);
        assert!(mixer.faulted());
        assert_eq!(mixer.frame(), 2);
    }
}
#[cfg(test)]
mod meter_tap_tests {
    use super::*;
    #[test]
    fn disabled_saturated_and_lost_meter_workers_leave_every_audio_and_operator_sample_exact() {
        for (inputs, monitors) in [(16, 3), (17, 1), (32, 5), (48, 7)] {
            let topology = crate::topology::EngineTopology::software(inputs, monitors, 0).unwrap();
            let mut reference = Mixer::from_topology(0, topology.clone()).unwrap();
            reference.meter_processed.clear();
            let mut actual = Mixer::from_topology(0, topology).unwrap();
            let (mut tap, latest) = crate::metering::prepare(inputs, monitors, 48000).unwrap();
            drop(latest);
            let edits = vec![
                Edit {
                    target: Target::Fader {
                        input: format!("input-{inputs:02}"),
                    },
                    value: Value::Integer(-12000),
                },
                Edit {
                    target: Target::Pan {
                        input: "input-01".into(),
                    },
                    value: Value::Integer(100),
                },
            ];
            for mixer in [&mut actual, &mut reference] {
                mixer
                    .schedule(Prepared::edits_for(inputs, monitors, &edits).unwrap(), 1)
                    .unwrap();
                mixer.set_operator_tap(crate::brain_control::MonitorSource::Pfl {
                    input: inputs - 1,
                });
            }
            let mut a = vec![0.; 48 * (monitors + 2)];
            let mut b = a.clone();
            let mut main = [0.; 96];
            let source = vec![0.5; 48 * inputs];
            for block in 0..100 {
                actual.process_interleaved(&source, &mut a).unwrap();
                reference.process_interleaved(&source, &mut b).unwrap();
                assert_eq!(a, b);
                assert_eq!(actual.operator_samples, reference.operator_samples);
                for f in 0..48 {
                    main[f * 2..f * 2 + 2]
                        .copy_from_slice(&a[f * (monitors + 2)..f * (monitors + 2) + 2]);
                }
                let (processed, valid) = actual.meter_samples();
                assert!(processed.iter().all(|v| *v == 0.5));
                tap.offer(
                    block * 48,
                    block,
                    &source,
                    processed,
                    valid,
                    &main,
                    &a,
                    true,
                );
                actual.take_completion();
                reference.take_completion();
            }
            assert!(tap.losses > 0);
        }
    }
    #[test]
    fn shared_mute_is_measured_zero_without_erasing_actual_pre_fader_sample() {
        let mut m = Mixer::default();
        m.schedule(
            Prepared::edits(&[Edit {
                target: Target::Mute {
                    input: "input-01".into(),
                },
                value: Value::Boolean(true),
            }])
            .unwrap(),
            1,
        )
        .unwrap();
        let mut source = [0.; 48 * 8];
        for f in 0..48 {
            source[f * 8] = 0.5;
        }
        let mut output = [0.; 48 * 4];
        for _ in 0..40 {
            m.process_interleaved(&source, &mut output).unwrap();
            m.take_completion();
        }
        assert!(output.iter().all(|v| *v == 0.));
        let (processed, valid) = m.meter_samples();
        assert!(valid.iter().all(|v| *v));
        assert!((0..48).all(|f| processed[f * 8] == 0.5));
    }
}
