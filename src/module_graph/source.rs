//! Actual PA/local-FX/raw-REC render storage. Controller lifecycle and protocol
//! metadata remain in ModuleGraph. This extraction does not start a source thread.
use super::{FRAMES, PaStorage, ProcessError};
use crate::host::adapters::{Dsp, Recorder};

pub(crate) struct ModuleSource {
    pub closure: std::sync::Arc<std::sync::atomic::AtomicU32>,
    pub library_identity: [u8; 64],
    pub fx: Dsp,
    pub pa: Dsp,
    pub processed: u64,
    pub source_fault: u32,
    pub epoch: u64,
    pub next: u64,
    pub wet: [f64; FRAMES * 2],
    pub sum: [f64; FRAMES * 2],
    pub program: [f64; FRAMES * 2],
    pub raw: Vec<f64>,
    pub output: [f64; FRAMES * 6],
    pub inputs: usize,
    pub pa_v2: Option<crate::host::pa_v2::Pa>,
    pub pa_storage: Option<PaStorage>,
    pub pa_retired_storage: Option<PaStorage>,
    pub pa_hold: bool,
    pub recorder: Option<Recorder>,
    pub recording: bool,
    pub recorder_cancel_code: u32,
}
/// Fixed-size numeric source observation; safe to retain while the actual DSP
/// continues rendering. Configuration strings and lifecycle workers are excluded.
#[derive(Clone, Debug)]
pub(crate) struct ModuleObservation {
    pub epoch: u64,
    pub next: u64,
    pub processed: u64,
    pub inputs: usize,
    pub source_fault: u32,
    pub pa_hold: bool,
    pub pa_inputs: usize,
    pub pa_outputs: usize,
    pub fx_status: Result<crate::host::adapters::FxStatus, i32>,
    pub pa_status: Result<crate::host::adapters::PaStatus, i32>,
    pub pa_v2_status: Option<Result<crate::host::pa_v2::Status, i32>>,
    pub pa_v2_capabilities: Option<crate::host::pa_v2::Capabilities>,
    pub eq_status: Option<Result<crate::host::pa_v2::EqStatus, i32>>,
    pub pa_retirement_occupied: bool,
    pub recorder_present: bool,
    pub recorder_cancel_code: u32,
}
impl ModuleSource {
    /// One bounded command/observation transfer. All refused and retired owner
    /// storage remains in the caller's packet for off-source destruction.
    pub(crate) fn exchange(&mut self, packet: &mut super::ModuleExchange) {
        if packet.serviced {
            return;
        }
        let closure = self.closure.load(std::sync::atomic::Ordering::Acquire);
        if closure != 0 {
            self.mark_source_fault(closure);
        }
        if packet.epoch != self.epoch || !std::sync::Arc::ptr_eq(&self.closure, &packet.owner) {
            // A stale packet may retire its own prepared handle, never stop or
            // replace a recorder belonging to the new source epoch.
            if let Some(recorder) = &mut packet.start {
                recorder.fault(6);
            }
            packet.serviced = true;
            return;
        }
        packet.accepted = true;
        let valid = self.source_fault == 0;
        if packet.stop || !valid {
            self.recording = false;
            packet.retired_recorder = self.recorder.take();
            if !valid && let Some(recorder) = &mut packet.retired_recorder {
                recorder.fault(if self.recorder_cancel_code != 0 {
                    self.recorder_cancel_code
                } else {
                    6
                });
            }
            if let Some(recorder) = &mut packet.start {
                recorder.fault(if self.recorder_cancel_code != 0 {
                    self.recorder_cancel_code
                } else {
                    5
                });
            }
        } else if self.recorder.is_none() && packet.start.is_some() {
            self.recorder_cancel_code = 0;
            self.recorder = packet.start.take();
            self.recording = true;
            packet.activated_frame = Some(self.next);
        } else if let Some(recorder) = &mut packet.start {
            recorder.fault(5);
        }
        if let (Some(pa), Some(slot)) = (&mut self.pa_v2, &mut packet.pa_retirement)
            && pa.take_retired(slot) == 0
        {
            packet.retired_storage = self.pa_retired_storage.take();
        }
        if let Some(observation) = &mut packet.pa {
            // An occupied native return remains retained. A later controller
            // packet supplies an empty slot; no render-side disposal occurs.
            if observation.retirement.is_empty() {
                let _ = self.return_eq_retirement(observation);
            }
            self.capture_pa_observation(observation);
        }
        packet.observation = self.observation();
        packet.serviced = true;
    }
    /// Complete the same retained packet after rendering, without repeating its
    /// commands. A full return lane keeps this end-frame snapshot unchanged.
    pub(crate) fn observe_exchange(&self, packet: &mut super::ModuleExchange) {
        if !packet.serviced
            || !packet.accepted
            || packet.epoch != self.epoch
            || !std::sync::Arc::ptr_eq(&self.closure, &packet.owner)
        {
            return;
        }
        packet.observation = self.observation();
        if let Some(observation) = &mut packet.pa {
            self.capture_pa_observation(observation);
        }
    }

    pub fn output_interleaved(&self) -> &[f64] {
        self.pa_storage
            .as_ref()
            .map_or(&self.output, |s| s.output.as_slice())
    }

    pub fn observation(&self) -> ModuleObservation {
        ModuleObservation {
            epoch: self.epoch,
            next: self.next,
            processed: self.processed,
            inputs: self.inputs,
            source_fault: self.source_fault,
            pa_hold: self.pa_hold,
            pa_inputs: self.pa_v2.as_ref().map_or(2, |p| p.input_channels()),
            pa_outputs: self.pa_v2.as_ref().map_or(6, |p| p.output_channels()),
            fx_status: self.fx.fx_status_numeric(),
            pa_status: self.pa.pa_status_numeric(),
            pa_v2_status: self.pa_v2.as_ref().map(|p| p.status()),
            pa_v2_capabilities: self.pa_v2.as_ref().map(|p| p.capabilities()),
            eq_status: self.pa_v2.as_ref().map(|p| p.eq_status()),
            pa_retirement_occupied: self.pa_retired_storage.is_some(),
            recorder_present: self.recorder.is_some(),
            recorder_cancel_code: self.recorder_cancel_code,
        }
    }
    /// Apply only numeric/native state. The caller retains the prepared packet
    /// and old storage on both success and refusal for controller destruction.
    pub fn commit_pa_change(
        &mut self,
        prepared: &mut super::GraphPaPrepared,
        epoch: u64,
        frame: u64,
    ) -> i32 {
        let closure = self.closure.load(std::sync::atomic::Ordering::Acquire);
        if closure != 0 {
            self.mark_source_fault(closure);
            return -2;
        }
        if prepared.storage.is_none()
            || prepared.expected_library_sha256.as_bytes() != self.library_identity
        {
            return -1;
        }
        if self.pa_retired_storage.is_some() {
            return -3;
        }
        if epoch != self.epoch || epoch != prepared.expected_epoch || frame != self.next {
            return -4;
        }
        if self.source_fault != 0 {
            return -2;
        }
        if let Some(pa) = &self.pa_v2 {
            let status = match pa.status() {
                Ok(s) => s,
                Err(e) => return e,
            };
            if status.generation != prepared.expected_generation {
                return -4;
            }
        } else if self.processed != 0 && !self.pa_hold {
            return -3;
        }
        let result = if let Some(pa) = &mut self.pa_v2 {
            pa.commit(&mut prepared.owner, epoch, frame)
        } else if let Some(pa) = &mut prepared.initial {
            pa.commit(&mut prepared.owner, epoch, frame)
        } else {
            return -1;
        };
        if result != 0 {
            return result;
        }
        if self.pa_v2.is_none() {
            self.pa_v2 = prepared.initial.take();
        }
        self.pa_retired_storage = self.pa_storage.take();
        self.pa_storage = prepared.storage.take();
        self.pa_hold = true;
        0
    }
    pub fn commit_master_eq(
        &mut self,
        p: &mut super::GraphEqPrepared,
        epoch: u64,
        frame: u64,
    ) -> i32 {
        let closure = self.closure.load(std::sync::atomic::Ordering::Acquire);
        if closure != 0 {
            self.mark_source_fault(closure);
            return -2;
        }
        if self.source_fault != 0 {
            return -2;
        }
        if epoch != self.epoch
            || frame != self.next
            || p.expected_map
                != self
                    .pa_storage
                    .as_ref()
                    .map_or(&[][..], |s| s.program_buses.as_slice())
        {
            return -4;
        }
        let Some(storage) = self.pa_storage.as_mut() else {
            return -5;
        };
        let Some(pa) = self.pa_v2.as_mut() else {
            return -5;
        };
        let code = pa.commit_eq(&mut p.owner, epoch, frame);
        if code == 0 {
            std::mem::swap(&mut storage.configuration_json, &mut p.metadata);
        }
        code
    }
    pub fn mute_pa(&mut self) -> i32 {
        let result = self.pa_v2.as_mut().map_or(0, |pa| pa.mute());
        if result == 0 {
            self.pa_hold = true;
        }
        result
    }
    pub fn rearm_pa(&mut self) -> i32 {
        let closure = self.closure.load(std::sync::atomic::Ordering::Acquire);
        if closure != 0 {
            self.mark_source_fault(closure);
            return -2;
        }
        if self.source_fault != 0 {
            return -2;
        }
        let result = self
            .pa_v2
            .as_mut()
            .map_or(0, |pa| pa.rearm(self.epoch, self.next));
        if result == 0 {
            self.pa_hold = false;
        }
        result
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn process_interleaved_brain(
        &mut self,
        epoch: u64,
        frame: u64,
        raw: &[f64],
        mixed: &[f64],
        buses: usize,
        external_wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
    ) -> std::result::Result<(), ProcessError> {
        self.process_with_master(
            epoch,
            frame,
            raw,
            mixed,
            buses,
            external_wet,
            talkback,
            &[1.; 48],
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn process_with_master(
        &mut self,
        epoch: u64,
        frame: u64,
        raw: &[f64],
        mixed: &[f64],
        buses: usize,
        external_wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
        master: &[f64; 48],
    ) -> std::result::Result<(), ProcessError> {
        self.output.fill(0.);
        if let Some(storage) = &mut self.pa_storage {
            storage.output.fill(0.);
        }
        let closure = self.closure.load(std::sync::atomic::Ordering::Acquire);
        if closure != 0 {
            self.mark_source_fault(closure);
            return Err(ProcessError::Source);
        }
        if raw.len() != FRAMES * self.inputs
            || buses < 2
            || buses.checked_mul(FRAMES) != Some(mixed.len())
            || external_wet.is_some_and(|w| w.len() != FRAMES * 2)
            || talkback.is_some_and(|v| v.len() != mixed.len() || v.iter().any(|x| !x.is_finite()))
            || self
                .pa_storage
                .as_ref()
                .is_some_and(|s| buses < s.total_buses)
        {
            self.mark_source_fault(7);
            return Err(ProcessError::Source);
        }
        if epoch != self.epoch || frame != self.next || frame.checked_add(48).is_none() {
            self.mark_source_fault(6);
            return Err(ProcessError::Timeline);
        }
        if raw.iter().any(|v| {
            !v.is_finite()
                || *v < -1.
                || *v > 8_388_607. / 8_388_608.
                || (v * 8_388_608.).fract() != 0.
        }) {
            self.mark_source_fault(7);
            return Err(ProcessError::Source);
        }
        for i in 0..FRAMES {
            self.raw[i * self.inputs..(i + 1) * self.inputs]
                .copy_from_slice(&raw[i * self.inputs..(i + 1) * self.inputs]);
            self.sum[i * 2..i * 2 + 2].copy_from_slice(&mixed[i * buses..i * buses + 2]);
        }
        if self.recording
            && let Some(r) = &mut self.recorder
        {
            let _ = r.push(frame, &self.raw);
        }
        let fx = if let Some(wet) = external_wet {
            self.wet.copy_from_slice(wet);
            0
        } else {
            self.fx.process_result(&self.sum, &mut self.wet, 2)
        };
        if fx != 0 {
            self.wet.fill(0.);
        }
        for i in 0..FRAMES * 2 {
            self.sum[i] = (self.sum[i] + self.wet[i]) * master[i / 2];
        }
        self.program.copy_from_slice(&self.sum);
        if let Some(tb) = talkback {
            for f in 0..FRAMES {
                self.sum[f * 2] += tb[f * buses];
                self.sum[f * 2 + 1] += tb[f * buses + 1];
            }
        }
        let pa = if let (Some(pa), Some(storage)) = (&mut self.pa_v2, &mut self.pa_storage) {
            let count = storage.program_buses.len();
            for i in 0..FRAMES {
                for (channel, &bus) in storage.program_buses.iter().enumerate() {
                    // Main buses include the actual wet return before owner
                    // summing/crossover/protection; monitor buses remain explicit.
                    storage.input[i * count + channel] = if bus < 2 {
                        self.sum[i * 2 + bus]
                    } else {
                        mixed[i * buses + bus] + talkback.map_or(0., |tb| tb[i * buses + bus])
                    };
                }
            }
            pa.process(&storage.input, &mut storage.output, epoch, frame)
        } else {
            let result = self.pa.process_result(&self.sum, &mut self.output, 6);
            if self.pa_hold {
                self.output.fill(0.);
            }
            result
        };
        if pa != 0 {
            self.output.fill(0.);
            if let Some(storage) = &mut self.pa_storage {
                storage.output.fill(0.);
            }
        }
        self.next += 48;
        self.processed = self.processed.saturating_add(1);
        if pa != 0 {
            Err(ProcessError::Pa(pa))
        } else if fx != 0 {
            Err(ProcessError::Fx(fx))
        } else {
            Ok(())
        }
    }
    /// Copies native fixed-size EQ state into preallocated exclusively owned
    /// storage. No JSON, allocation, destruction or lifecycle operation occurs.
    pub(crate) fn capture_pa_observation(&self, into: &mut super::PaEqObservation) {
        into.pa_status = self.pa_v2.as_ref().map_or(Err(-5), |pa| pa.status());
        into.result = match (&self.pa_v2, into.indices) {
            (Some(pa), Some(indices)) => pa.capture_eq_context(&mut into.context, indices),
            _ => -5,
        };
    }
    /// Return native retired ownership through the already reserved packet.
    /// The controller destroys it only after source ownership has been returned.
    pub(crate) fn return_eq_retirement(&mut self, into: &mut super::PaEqObservation) -> i32 {
        if !into.retirement.is_empty() {
            return -3;
        }
        let Some(pa) = &mut self.pa_v2 else {
            return -5;
        };
        let status = match pa.eq_status() {
            Ok(s) => s,
            Err(code) => return code,
        };
        let code = if status.retirement_occupied == 0 {
            0
        } else {
            pa.take_retired_eq(&mut into.retirement)
        };
        if code == 0 {
            self.capture_pa_observation(into);
        }
        code
    }
    pub fn mark_source_fault(&mut self, code: u32) {
        if self.source_fault == 0 {
            self.source_fault = code;
        }
        if self.recorder_cancel_code == 0 {
            self.recorder_cancel_code = code;
        }
        if let Some(r) = &mut self.recorder {
            r.fault(code);
        }
    }
}
