//! Source-owned routing buffers and bounded sample operations. Controller authority,
//! protocol state, telemetry readback and lifecycle workers deliberately live outside
//! this storage. HostAuthority retains the graph across production quanta;
//! controller service and scheduling are still synchronous.
use crate::{brain_control::MonitorSource, topology::EngineTopology};

/// Controller readback copied at an explicitly completed source boundary.
pub(super) struct RoutingObservation {
    pub monitor: [f64; 96],
    pub send: [f64; 96],
    pub aux: [[f64; 96]; 4],
    pub buses: Vec<f64>,
    pub meter_losses: u64,
}
impl RoutingObservation {
    pub fn new(topology: &EngineTopology) -> Self {
        Self {
            monitor: [0.; 96],
            send: [0.; 96],
            aux: [[0.; 96]; 4],
            buses: vec![0.; 48 * (topology.monitors + 2)],
            meter_losses: 0,
        }
    }
}

pub(crate) struct SourceRouting {
    pub analysis: Option<(
        crate::analysis_stream::Tap,
        crate::analysis_stream::local::SourceSignal,
        u64,
    )>,
    pub spectrum: Box<crate::spectrum::Acquisition>,
    pub measurement: Option<crate::measurement_capture::CaptureSource>,
    pub metering: Option<crate::metering::Acquisition>,
    pub meter_main: [f64; 96],
    pub meter_map: u64,
    pub brain_monitor: [f64; 96],
    pub brain_send: [f64; 96],
    pub brain_talkback: Vec<f64>,
    talkback_envelope: f64,
    pub raw_scratch: Vec<f64>,
    pub bus_scratch: Vec<f64>,
    pub pa_silence: Vec<f64>,
    pub capture_scratch: Vec<f64>,
    pub playback_scratch: Vec<f64>,
    pub pcm_scratch: Vec<i32>,
}
impl SourceRouting {
    #[cfg(test)]
    pub fn new(topology: &EngineTopology, metering: Option<crate::metering::Acquisition>) -> Self {
        Self::with_spectrum(
            topology,
            metering,
            crate::spectrum::Spectrum::new(topology.sample_rate)
                .take_acquisition()
                .unwrap(),
        )
    }
    pub fn with_spectrum(
        topology: &EngineTopology,
        metering: Option<crate::metering::Acquisition>,
        spectrum: Box<crate::spectrum::Acquisition>,
    ) -> Self {
        Self {
            spectrum,
            analysis: None,
            measurement: None,
            metering,
            meter_main: [0.; 96],
            meter_map: topology.map_revision,
            brain_monitor: [0.; 96],
            brain_send: [0.; 96],
            brain_talkback: vec![0.; 48 * (topology.monitors + 2)],
            talkback_envelope: 0.,
            raw_scratch: vec![0.; 48 * topology.inputs.len()],
            bus_scratch: vec![0.; 48 * (topology.monitors + 2)],
            pa_silence: vec![0.; 48 * topology.pa_outputs],
            capture_scratch: vec![0.; 48 * topology.capture_channels],
            playback_scratch: vec![0.; 48 * topology.playback_channels],
            pcm_scratch: vec![0; 48 * topology.inputs.len()],
        }
    }
    /// Source acquisition only. All transport, FFT, serialization and joining
    /// remain with controller-owned services. Fault closure cannot fill a queue.
    pub fn offer_analysis(
        &mut self,
        inputs: &[f64],
        channels: usize,
        frame: u64,
        received: Option<u64>,
        map_revision: u64,
    ) {
        if let Some(mono) = received {
            self.spectrum
                .offer(inputs, channels, frame, mono, map_revision);
        }
        if let Some((tap, signal, map)) = &mut self.analysis {
            if *map != map_revision
                || inputs.len() != self.pcm_scratch.len()
                || inputs
                    .iter()
                    .any(|v| !v.is_finite() || *v < -1. || *v > 8_388_607. / 8_388_608.)
            {
                signal.stop();
            } else if signal.alive() {
                if let Some(mono) = received {
                    for (out, v) in self.pcm_scratch.iter_mut().zip(inputs) {
                        *out = (v * 8_388_608.).round() as i32;
                    }
                    if tap
                        .offer_interleaved(frame, &self.pcm_scratch, channels, mono)
                        .is_err()
                    {
                        signal.stop();
                    }
                    signal.update_losses(tap.dropped_windows);
                } else {
                    signal.stop();
                }
            }
        }
    }
    pub fn close_brain_audio(&mut self) {
        self.talkback_envelope = 0.;
        self.brain_monitor.fill(0.);
        self.brain_talkback.fill(0.);
    }
    pub fn render_talkback(
        &mut self,
        view: &crate::brain_control::TalkbackRender<'_>,
        samples: Option<&[f64]>,
        buses: usize,
    ) -> (f64, f64) {
        crate::brain_control::render_talkback_block(
            view,
            &mut self.talkback_envelope,
            samples,
            &mut self.brain_talkback,
            buses,
        )
        .unwrap_or((0., 0.))
    }
    pub fn capture_inputs(topology: &EngineTopology, capture: &[f64], raw: &mut [f64]) {
        for f in 0..48 {
            for (i, p) in topology.inputs.iter().enumerate() {
                raw[f * topology.inputs.len() + i] =
                    capture[f * topology.capture_channels + p.capture_slot];
            }
        }
    }
    pub fn capture_send(&mut self, buses: usize, output: &[f64]) {
        for f in 0..48 {
            self.brain_send[f * 2..f * 2 + 2].copy_from_slice(&output[f * buses..f * buses + 2]);
        }
    }
    /// Tap before talkback; PFL/AFL retains the mixer's source-frame ring indexing.
    pub fn capture_monitor(
        &mut self,
        source: MonitorSource,
        first_frame: u64,
        buses: usize,
        output: &[f64],
        operator: &[f64],
    ) {
        for f in 0..48 {
            let pair = match source {
                MonitorSource::None => [0.; 2],
                MonitorSource::Main => [output[f * buses], output[f * buses + 1]],
                MonitorSource::Monitor { index } => {
                    [output[f * buses + 2 + index] * std::f64::consts::FRAC_1_SQRT_2; 2]
                }
                _ => {
                    let i = ((first_frame + f as u64) % 48) as usize * 2;
                    [operator[i], operator[i + 1]]
                }
            };
            self.brain_monitor[f * 2..f * 2 + 2].copy_from_slice(&pair);
            self.meter_main[f * 2..f * 2 + 2].copy_from_slice(&output[f * buses..f * buses + 2]);
        }
    }
    pub fn mix_monitor_talkback(&self, buses: usize, output: &mut [f64]) {
        for (out, tb) in output
            .chunks_exact_mut(buses)
            .zip(self.brain_talkback.chunks_exact(buses))
        {
            for i in 2..buses {
                out[i] += tb[i];
            }
        }
    }
}

/// Exclusive source graph. Production retains this value across quanta;
/// offline compatibility callers may reattach after their synchronous pump.
pub(crate) struct RenderGraph {
    pub mixer: crate::mixer_control::source::SourceState,
    pub routing: SourceRouting,
    pub closure: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(feature = "hardware-host")]
    pub clock_routing: Option<ClockRouting>,
    #[cfg(feature = "hardware-host")]
    pub modules: Option<crate::module_graph::source::ModuleSource>,
    #[cfg(feature = "hardware-host")]
    pub(super) aux: Option<super::fx_aux::Rack>,
}

/// The synchronous controller's routing slot is empty during exclusive transfer.
/// It is not a shared-state or locking mechanism.
pub(super) struct RoutingSlot(Option<SourceRouting>);
impl RoutingSlot {
    pub fn new(source: SourceRouting) -> Self {
        Self(Some(source))
    }
    pub fn take(&mut self) -> SourceRouting {
        self.0.take().expect("routing already transferred")
    }
    pub fn restore(&mut self, source: SourceRouting) {
        assert!(self.0.is_none(), "routing already attached");
        self.0 = Some(source);
    }
}
impl std::ops::Deref for RoutingSlot {
    type Target = SourceRouting;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("routing transferred")
    }
}
impl std::ops::DerefMut for RoutingSlot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("routing transferred")
    }
}
/// Copied, bounded authorization for autonomous rendering. Neither wall-clock
/// expiry nor a source-frame deadline is extended by a stalled controller.
#[cfg(feature = "hardware-host")]
pub(crate) struct ClockRouting {
    pub monitor: MonitorSource,
    pub selection_generation: u64,
    pub monitor_silent: bool,
    pub hold_generation: Option<u64>,
    pub hold_fence: Option<crate::remote::PolicyFence>,
    pub foh_fence: Option<crate::remote::PolicyFence>,
    pub deadline_frame: Option<u64>,
    pub deadline_ms: Option<u64>,
    pub monitors: Vec<usize>,
    pub foh: bool,
    pub gain: f64,
    pub remote_fx_aux: bool,
}
pub(crate) struct RenderIntent<'a> {
    pub monitor: MonitorSource,
    pub deadline: Option<u64>,
    pub monitors: &'a [usize],
    pub foh: bool,
    pub talkback_gain: f64,
    #[cfg(feature = "hardware-host")]
    pub remote_fx_aux: bool,
}
pub(crate) struct RenderPeaks {
    pub microphone: f64,
    pub outgoing: f64,
    pub monitor: f64,
}
#[derive(Debug)]
pub(crate) enum RenderError {
    Mixer,
    Closed,
    Mapping,
    #[cfg(feature = "hardware-host")]
    Module(crate::module_graph::ProcessError),
}

/// Reusable owned quantum: every allocating/native field travels back to the
/// controller. The source only borrows or swaps its contents.
pub(super) struct Quantum {
    pub epoch: u64,
    pub frame: u64,
    pub capture: bool,
    pub received: Option<u64>,
    pub input: Vec<f64>,
    pub output: Vec<f64>,
    pub wet: Vec<f64>,
    pub talkback: Vec<f64>,
    pub monitor: MonitorSource,
    pub deadline: Option<u64>,
    pub monitors: Vec<usize>,
    pub foh: bool,
    pub gain: f64,
    #[cfg(feature = "hardware-host")]
    pub remote_fx_aux: bool,
    pub command: Option<crate::mixer_control::source::SourceCommand>,
    pub completion: crate::mixer_control::source::SourceCompletion,
    pub routing: RoutingObservation,
    #[cfg(feature = "hardware-host")]
    pub modules: Option<crate::module_graph::ModuleExchange>,
    pub result: Option<Result<RenderPeaks, RenderError>>,
    pub refused: bool,
}
#[cfg(feature = "hardware-host")]
pub(super) struct ClockCommand {
    pub frame: u64,
    pub command: crate::mixer_control::source::SourceCommand,
    pub completion: crate::mixer_control::source::SourceCompletion,
    pub routing: RoutingObservation,
    pub installed: bool,
    pub refused: bool,
}
#[cfg(feature = "hardware-host")]
impl ClockCommand {
    pub fn advance(graph: &mut RenderGraph, packet: &mut Self) -> bool {
        let closed = graph.closure.load(std::sync::atomic::Ordering::Acquire);
        if !packet.installed {
            if closed
                || packet
                    .command
                    .authority_fence
                    .as_ref()
                    .is_some_and(|f| !f.valid())
                || graph.mixer.mixer.frame() > packet.frame
                || packet
                    .command
                    .expires
                    .is_some_and(|d| std::time::Instant::now() >= d)
            {
                packet.refused = true;
                return true;
            }
            if graph.mixer.mixer.frame() < packet.frame {
                return false;
            }
            if graph.mixer.install_command(&mut packet.command).is_err() {
                packet.refused = true;
                return true;
            }
            packet.installed = true;
        }
        if closed {
            graph.mixer.finish_closed_command();
        }
        if !graph.mixer.publish_completion(&mut packet.completion) {
            return false;
        }
        graph.observe_routing(&mut packet.routing);
        true
    }
}
impl Quantum {
    pub fn new(
        topology: &EngineTopology,
        completion: crate::mixer_control::source::SourceCompletion,
    ) -> Self {
        Self {
            epoch: 0,
            frame: 0,
            capture: false,
            received: None,
            input: Vec::with_capacity(48 * topology.capture_channels.max(topology.inputs.len())),
            output: Vec::with_capacity(48 * topology.playback_channels.max(topology.monitors + 2)),
            wet: Vec::with_capacity(384),
            talkback: Vec::with_capacity(48),
            monitor: MonitorSource::None,
            deadline: None,
            monitors: Vec::with_capacity(topology.monitors),
            foh: false,
            gain: 0.,
            #[cfg(feature = "hardware-host")]
            remote_fx_aux: false,
            command: None,
            completion,
            routing: RoutingObservation::new(topology),
            #[cfg(feature = "hardware-host")]
            modules: None,
            result: None,
            refused: false,
        }
    }
    pub fn execute(graph: &mut RenderGraph, packet: &mut Self) {
        packet.refused = false;
        if graph
            .mixer
            .install_command(packet.command.as_mut().expect("quantum command"))
            .is_err()
        {
            packet.refused = true;
            packet.result = Some(Err(RenderError::Mixer));
            return;
        }
        #[cfg(feature = "hardware-host")]
        if let (Some(source), Some(exchange)) = (&mut graph.modules, &mut packet.modules) {
            source.exchange(exchange);
        }
        let intent = RenderIntent {
            monitor: packet.monitor,
            deadline: packet.deadline,
            monitors: &packet.monitors,
            foh: packet.foh,
            talkback_gain: packet.gain,
            #[cfg(feature = "hardware-host")]
            remote_fx_aux: packet.remote_fx_aux,
        };
        let wet = (!packet.wet.is_empty()).then_some(packet.wet.as_slice());
        let talkback = (!packet.talkback.is_empty()).then_some(packet.talkback.as_slice());
        packet.result = Some(if packet.capture {
            graph.render_capture(
                packet.epoch,
                packet.frame,
                &packet.input,
                &mut packet.output,
                wet,
                talkback,
                packet.received,
                intent,
            )
        } else {
            graph.render(
                packet.epoch,
                &packet.input,
                &mut packet.output,
                wet,
                talkback,
                packet.received,
                intent,
            )
        });
        #[cfg(feature = "hardware-host")]
        if let (Some(source), Some(exchange)) = (&mut graph.modules, &mut packet.modules) {
            source.observe_exchange(exchange);
        }
        graph.observe_routing(&mut packet.routing);
        graph.mixer.publish_completion(&mut packet.completion);
    }
}
impl RenderGraph {
    pub(super) fn observe_routing(&self, observation: &mut RoutingObservation) {
        observation.monitor = self.routing.brain_monitor;
        observation.send = self.routing.brain_send;
        observation.aux = *self.mixer.mixer.fx_aux_samples();
        observation.buses.copy_from_slice(&self.routing.bus_scratch);
        observation.meter_losses = self.routing.metering.as_ref().map_or(0, |tap| tap.losses);
    }
    pub(crate) fn close_numeric(&mut self) {
        self.closure
            .store(true, std::sync::atomic::Ordering::Release);
        self.mixer.mixer.quiesce();
        self.routing.close_brain_audio();
        #[cfg(feature = "hardware-host")]
        if let Some(modules) = &mut self.modules {
            modules.mark_source_fault(6);
        }
    }
    /// Apply one controller-validated structural operation. The packet retains
    /// prepared and replaced owners until the controller receives the outcome.
    pub(super) fn apply_structure(
        &mut self,
        permit: &crate::mixer_control::source::ExternalPermit,
        action: &mut super::StructuralAction,
        epoch: u64,
        frame: u64,
        rearm_allowed: bool,
    ) -> crate::mixer_control::source::ExternalReceipt {
        #[cfg(feature = "hardware-host")]
        let modules = &mut self.modules;
        #[cfg(not(feature = "hardware-host"))]
        let _ = (epoch, frame);
        let closure = &self.closure;
        self.mixer.commit_external(permit, |mixer| {
            if closure.load(std::sync::atomic::Ordering::Acquire) {
                return -15;
            }
            match action {
                super::StructuralAction::Failed(_) => -10,
                super::StructuralAction::Patch(prepared) => {
                    if mixer.apply_output_patch(prepared).is_ok() {
                        0
                    } else {
                        -11
                    }
                }
                super::StructuralAction::Mute => {
                    if mixer.mute_outputs().is_err() {
                        return -12;
                    }
                    #[cfg(feature = "hardware-host")]
                    if let Some(modules) = modules {
                        return modules.mute_pa();
                    }
                    0
                }
                super::StructuralAction::Rearm => {
                    if !rearm_allowed
                        || mixer.faulted()
                        || mixer
                            .frame()
                            .checked_add(crate::mixer::RAMP_FRAMES)
                            .is_none()
                    {
                        return -13;
                    }
                    #[cfg(feature = "hardware-host")]
                    if let Some(modules) = modules {
                        let code = modules.rearm_pa();
                        if code != 0 {
                            return code;
                        }
                    }
                    if mixer.rearm().is_ok() { 0 } else { -13 }
                }
                #[cfg(feature = "hardware-host")]
                super::StructuralAction::MasterEq(prepared) => {
                    modules.as_mut().map_or(-14, |modules| {
                        modules.commit_master_eq(prepared, epoch, frame)
                    })
                }
                #[cfg(feature = "hardware-host")]
                super::StructuralAction::Pa(prepared) => modules.as_mut().map_or(-14, |modules| {
                    modules.commit_pa_change(prepared, epoch, frame)
                }),
            }
        })
    }
    /// Mapping, measurement acquisition and output-spectrum acquisition belong to
    /// the same exclusive owner as DSP. Scratch is restored on every error path.
    #[allow(clippy::too_many_arguments)]
    pub fn render_capture(
        &mut self,
        epoch: u64,
        first_frame: u64,
        capture: &[f64],
        playback: &mut [f64],
        wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
        received: Option<u64>,
        intent: RenderIntent<'_>,
    ) -> Result<RenderPeaks, RenderError> {
        playback.fill(0.);
        let topology = self.mixer.mixer.topology();
        if first_frame != self.mixer.mixer.frame()
            || capture.len() != 48 * topology.capture_channels
            || playback.len() != 48 * topology.playback_channels
        {
            self.close_numeric();
            return Err(RenderError::Mapping);
        }
        let mut raw = std::mem::take(&mut self.routing.raw_scratch);
        let mut buses = std::mem::take(&mut self.routing.bus_scratch);
        SourceRouting::capture_inputs(topology, capture, &mut raw);
        let result = self.render(epoch, &raw, &mut buses, wet, talkback, received, intent);
        if result.is_ok() {
            let topology = self.mixer.mixer.topology();
            if let Some(source) = &mut self.routing.measurement {
                source.offer(epoch, topology.map_revision, first_frame, capture, true);
            }
            #[cfg(feature = "hardware-host")]
            let pa = self
                .modules
                .as_ref()
                .map(|m| m.output_interleaved())
                .filter(|p| p.len() == self.routing.pa_silence.len())
                .unwrap_or(&self.routing.pa_silence);
            #[cfg(not(feature = "hardware-host"))]
            let pa = &self.routing.pa_silence;
            // Shape is admitted above and all graph dimensions are prepared.
            for (f, output) in playback
                .chunks_exact_mut(topology.playback_channels)
                .enumerate()
            {
                for patch in &topology.outputs {
                    output[patch.playback_slot] = match patch.source {
                        None => 0.,
                        Some(crate::topology::OutputSource::Main { channel }) => {
                            buses[f * (topology.monitors + 2) + channel]
                        }
                        Some(crate::topology::OutputSource::Monitor { index }) => {
                            buses[f * (topology.monitors + 2) + index + 2]
                        }
                        Some(crate::topology::OutputSource::Pa { index }) => {
                            pa[f * topology.pa_outputs + index]
                        }
                    };
                }
            }
            if let Some(mono) = received {
                self.routing.spectrum.offer_tap(
                    playback,
                    topology.playback_channels,
                    first_frame,
                    mono,
                    topology.map_revision,
                    crate::spectrum::OUTPUT,
                );
            }
        }
        self.routing.raw_scratch = raw;
        self.routing.bus_scratch = buses;
        result
    }
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        epoch: u64,
        inputs: &[f64],
        output: &mut [f64],
        wet: Option<&[f64]>,
        talkback: Option<&[f64]>,
        received: Option<u64>,
        intent: RenderIntent<'_>,
    ) -> Result<RenderPeaks, RenderError> {
        if self.closure.load(std::sync::atomic::Ordering::Acquire) {
            output.fill(0.);
            self.close_numeric();
            return Err(RenderError::Closed);
        }
        let frame = self.mixer.mixer.frame();
        let ni = self.mixer.mixer.input_count();
        let buses = self.mixer.mixer.monitor_count() + 2;
        let map = self.mixer.mixer.topology().map_revision;
        let routing = &mut self.routing;
        if routing.meter_map != map {
            if let Some(tap) = &mut routing.metering {
                tap.reset();
            }
            routing.meter_map = map;
        }
        routing.offer_analysis(inputs, ni, frame, received, map);
        if self.mixer.render(epoch, inputs, output).is_err() {
            self.closure
                .store(true, std::sync::atomic::Ordering::Release);
            self.mixer.mixer.quiesce();
            routing.close_brain_audio();
            #[cfg(feature = "hardware-host")]
            if let Some(modules) = &mut self.modules {
                modules.mark_source_fault(6);
            }
            return Err(RenderError::Mixer);
        }
        #[cfg(feature = "hardware-host")]
        let aux_wet = self
            .aux
            .as_mut()
            .map(|rack| rack.process(frame, &self.mixer.mixer));
        // Capture dry excitation before mixing any returned wet into monitors.
        routing.capture_send(buses, output);
        #[cfg(feature = "hardware-host")]
        let mut aux_main = [0.; 96];
        #[cfg(feature = "hardware-host")]
        let aux_input: Option<&[f64; 384]> = aux_wet.as_ref().or_else(|| {
            if intent.remote_fx_aux {
                wet.and_then(|w| w.try_into().ok())
            } else {
                None
            }
        });
        #[cfg(feature = "hardware-host")]
        if let Some(aux) = aux_input {
            self.mixer
                .mixer
                .mix_fx_returns(frame, aux, &mut aux_main, output);
        }
        #[cfg(feature = "hardware-host")]
        let wet = if aux_input.is_some() {
            Some(aux_main.as_slice())
        } else {
            wet
        };
        let safety = std::array::from_fn(|f| self.mixer.mixer.output_safety_gain(frame + f as u64));
        #[cfg(feature = "hardware-host")]
        let foh = intent.foh && self.modules.is_some();
        #[cfg(not(feature = "hardware-host"))]
        let foh = {
            let _ = intent.foh;
            false
        };
        let (microphone, outgoing) = routing.render_talkback(
            &crate::brain_control::TalkbackRender {
                first_frame: frame,
                deadline_frame: intent.deadline,
                monitors: intent.monitors,
                foh,
                gain: intent.talkback_gain,
                safety: &safety,
            },
            talkback,
            buses,
        );
        routing.capture_monitor(
            intent.monitor,
            frame,
            buses,
            output,
            self.mixer.mixer.operator_tap(),
        );
        #[allow(unused_mut)]
        let mut meter_bus_valid = true;
        #[cfg(feature = "hardware-host")]
        if let Some(graph) = &mut self.modules {
            match graph.process_interleaved_brain(
                epoch,
                frame,
                inputs,
                output,
                buses,
                wet,
                Some(&routing.brain_talkback),
            ) {
                Ok(()) => routing.meter_main.copy_from_slice(&graph.sum),
                Err(crate::module_graph::ProcessError::Fx(_)) => {
                    routing.meter_main.copy_from_slice(&graph.sum);
                    meter_bus_valid = false;
                }
                Err(error) => {
                    output.fill(0.);
                    self.closure
                        .store(true, std::sync::atomic::Ordering::Release);
                    self.mixer.mixer.quiesce();
                    routing.close_brain_audio();
                    return Err(RenderError::Module(error));
                }
            }
        }
        if matches!(intent.monitor, MonitorSource::Main) {
            #[cfg(feature = "hardware-host")]
            if let Some(graph) = &self.modules {
                routing.brain_monitor.copy_from_slice(&graph.program);
            } else if let Some(wet) = wet {
                for (out, v) in routing.brain_monitor.iter_mut().zip(wet) {
                    *out += v;
                }
            }
            #[cfg(not(feature = "hardware-host"))]
            if let Some(wet) = wet {
                for (out, v) in routing.brain_monitor.iter_mut().zip(wet) {
                    *out += v;
                }
            }
        }
        #[cfg(feature = "hardware-host")]
        if let Some(aux) = aux_input.map(|_| aux_main.as_slice()) {
            for (out, pair) in output.chunks_exact_mut(buses).zip(aux.chunks_exact(2)) {
                out[0] += pair[0];
                out[1] += pair[1];
            }
            if self.modules.is_none() {
                for (meter, wet) in routing.meter_main.iter_mut().zip(aux) {
                    *meter += wet;
                }
            }
        }
        routing.mix_monitor_talkback(buses, output);
        if let Some(tap) = &mut routing.metering
            && let Some(mono) = received
        {
            let (processed, valid) = self.mixer.mixer.meter_samples();
            tap.offer(
                frame,
                mono,
                inputs,
                processed,
                valid,
                &routing.meter_main,
                output,
                meter_bus_valid,
            );
        }
        if meter_bus_valid && let Some(mono) = received {
            routing.spectrum.offer_tap(
                &routing.meter_main,
                2,
                frame,
                mono,
                map,
                crate::spectrum::MAIN,
            );
        }
        Ok(RenderPeaks {
            microphone,
            outgoing,
            monitor: routing
                .brain_monitor
                .iter()
                .fold(0_f64, |peak, sample| peak.max(sample.abs())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_capture_owner_survives_stall_and_closes_without_controller() {
        use crate::{local_audio::LocalAudio, show::Counter};
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("gp-h1-source-map-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = Directory(directory);
        let mut topology = EngineTopology::software(8, 2, 0).unwrap();
        for (index, port) in topology.inputs.iter_mut().enumerate() {
            port.capture_slot = (index + 3) % 8;
        }
        for (index, port) in topology.outputs.iter_mut().enumerate() {
            port.playback_slot = (index + 1) % 4;
        }
        let make = |name: &str| {
            let root = directory.0.join(name);
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            LocalAudio::bind_configured(
                &root,
                name,
                // Separate durable epoch ledgers for the two independent providers.
                if name == "owner" {
                    "11111111-1111-4111-8111-111111111111"
                } else {
                    "22222222-2222-4222-8222-222222222222"
                },
                Counter(9),
                topology.clone(),
            )
            .unwrap()
        };
        let mut owner = make("owner");
        owner.enable_persistent_source();
        let mut reference = make("reference");
        owner.rearm().unwrap();
        reference.rearm().unwrap();
        let input: Vec<_> = (0..384).map(|i| ((i % 8) + 1) as f64 / 128.).collect();
        let mut expected = Vec::new();
        for block in 0..33 {
            let mut output = [0.; 192];
            reference
                .tick_with_capture(block, 9, block * 48, &input, &mut output)
                .unwrap();
            expected.push(output);
        }
        let mut initial = [0.; 192];
        owner
            .tick_with_capture(0, 9, 0, &input, &mut initial)
            .unwrap();
        assert_eq!(initial, expected[0]);
        assert!(
            owner.engine.take_source().is_err(),
            "production mixer must stay detached"
        );
        let graph_address = owner.render_graph.as_deref().unwrap() as *const RenderGraph;
        owner
            .tick_with_capture(1, 9, 48, &input, &mut initial)
            .unwrap();
        assert_eq!(initial, expected[1]);
        assert_eq!(
            owner.render_graph.as_deref().unwrap() as *const RenderGraph,
            graph_address
        );
        assert!(owner.engine.take_source().is_err());
        // Transfer the production-retained value for the existing stalled-controller
        // kernel check; this is not a runner scheduling acceptance claim.
        let mut graph = owner.render_graph.take().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
        let (closed_tx, closed_rx) = std::sync::mpsc::sync_channel(0);
        let worker = std::thread::spawn(move || {
            let intent = || RenderIntent {
                monitor: MonitorSource::None,
                deadline: None,
                monitors: &[],
                foh: false,
                talkback_gain: 1.,
                #[cfg(feature = "hardware-host")]
                remote_fx_aux: false,
            };
            let mut output = [0.; 192];
            crate::remote::assert_no_allocations(|| {
                for (block, wanted) in expected.iter().enumerate().skip(2) {
                    graph
                        .render_capture(
                            9,
                            block as u64 * 48,
                            &input,
                            &mut output,
                            None,
                            None,
                            None,
                            intent(),
                        )
                        .unwrap();
                    assert_eq!(&output, wanted);
                }
            });
            ready_tx.send(()).unwrap();
            closed_rx.recv().unwrap();
            crate::remote::assert_no_allocations(|| {
                assert!(matches!(
                    graph.render_capture(
                        9,
                        33 * 48,
                        &input,
                        &mut output,
                        None,
                        None,
                        None,
                        intent()
                    ),
                    Err(RenderError::Closed)
                ));
                assert!(output.iter().all(|sample| *sample == 0.));
            });
            graph
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        // No provider pump, free command slot or source reference is needed.
        drop(owner);
        closed_tx.send(()).unwrap();
        let graph = worker.join().unwrap();
        assert_eq!(graph.mixer.mixer.frame(), 33 * 48);
    }

    #[test]
    fn combined_render_keeps_talkback_taps_and_samples_without_controller_pump() {
        use crate::mixer_control::OfflineEngine;
        use crate::show::Counter;
        let make = || {
            OfflineEngine::new(
                "11111111-1111-4111-8111-111111111111",
                Counter(9),
                Counter(0),
                0,
            )
            .unwrap()
        };
        let mut engine = make();
        let mut reference = make();
        let routing = SourceRouting::new(engine.topology(), None);
        let input = [0.125; 384];
        let talkback = [0.25; 48];
        let mut expected = Vec::new();
        let mut envelope = 0.;
        for block in 0..32 {
            let frame = block * 48;
            let mut mixed = [0.; 192];
            reference
                .process_interleaved(&input, &mut mixed, 0)
                .unwrap();
            let mut taps = [0.; 96];
            for f in 0..48 {
                taps[f * 2..f * 2 + 2].copy_from_slice(&mixed[f * 4..f * 4 + 2]);
            }
            let safety = std::array::from_fn(|f| reference.output_safety_gain(frame + f as u64));
            let mut tb = [0.; 192];
            crate::brain_control::render_talkback_block(
                &crate::brain_control::TalkbackRender {
                    first_frame: frame,
                    deadline_frame: Some(15 * 48 + 17),
                    monitors: &[0],
                    foh: false,
                    gain: 0.5,
                    safety: &safety,
                },
                &mut envelope,
                Some(&talkback),
                &mut tb,
                4,
            )
            .unwrap();
            for f in 0..48 {
                for bus in 2..4 {
                    mixed[f * 4 + bus] += tb[f * 4 + bus];
                }
            }
            expected.push((mixed, taps));
        }
        engine.prepare_render(48, 0).unwrap();
        let graph = std::thread::scope(|scope| {
            let mixer = engine.take_source().unwrap();
            scope
                .spawn(move || {
                    let mut graph = RenderGraph {
                        #[cfg(feature = "hardware-host")]
                        clock_routing: None,
                        mixer,
                        routing,
                        closure: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        #[cfg(feature = "hardware-host")]
                        modules: None,
                        #[cfg(feature = "hardware-host")]
                        aux: None,
                    };
                    let mut output = [0.; 192];
                    crate::remote::assert_no_allocations(|| {
                        for (samples, taps) in &expected {
                            graph
                                .render(
                                    9,
                                    &input,
                                    &mut output,
                                    None,
                                    Some(&talkback),
                                    None,
                                    RenderIntent {
                                        monitor: MonitorSource::Main,
                                        deadline: Some(15 * 48 + 17),
                                        monitors: &[0],
                                        foh: false,
                                        talkback_gain: 0.5,
                                        #[cfg(feature = "hardware-host")]
                                        remote_fx_aux: false,
                                    },
                                )
                                .unwrap();
                            assert_eq!(&output, samples);
                            assert_eq!(&graph.routing.brain_monitor, taps);
                            assert_eq!(&graph.routing.brain_send, taps);
                        }
                    });
                    graph
                })
                .join()
                .unwrap()
        });
        engine.restore_source(graph.mixer).unwrap();
        assert_eq!(engine.frame(), 1536);
        engine.finish_render().unwrap();
    }

    /// Combined numeric production-kernel coverage. This does not establish
    /// independent HostAuthority/mailbox/device scheduling or global admission.
    #[cfg(feature = "hardware-host")]
    #[test]
    #[ignore = "actual PA/FX/REC bindings: GP05_MANIFEST and GP_PA_CONFIG"]
    fn actual_combined_render_pa_fx_raw_rec_without_controller_pump() {
        use crate::{
            mixer_control::OfflineEngine,
            module_graph::{Manifest, ModuleGraph},
            show::Counter,
        };
        use std::{
            path::{Path, PathBuf},
            time::Duration,
        };
        const SHOW: &str = "11111111-1111-4111-8111-111111111111";
        struct Directory(PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        // Declared before the owners: every lifecycle joins before cleanup,
        // including assertion unwinding. Never reuse/delete an existing path.
        let path =
            std::env::temp_dir().join(format!("gp-h1-combined-render-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let directory = Directory(path);
        let manifest =
            Manifest::load(Path::new(&std::env::var_os("GP05_MANIFEST").unwrap())).unwrap();
        let config = std::fs::read(std::env::var_os("GP_PA_CONFIG").unwrap()).unwrap();
        let make_mixer = || OfflineEngine::new(SHOW, Counter(9), Counter(0), 0).unwrap();
        let mut mixer = make_mixer();
        let mut reference_mixer = make_mixer();
        let make_modules = || {
            let mut graph = ModuleGraph::load(manifest.clone(), 9, 0).unwrap();
            graph.configure_pa_v2(&config).unwrap();
            graph.rearm_pa().unwrap();
            graph
        };
        let mut modules = make_modules();
        let mut reference_modules = make_modules();
        let routing = SourceRouting::new(mixer.topology(), None);
        let talkback = [0.25; 48];
        let deadline = Some(16 * 48 + 17);
        let mut envelope = 0.;
        let mut expected = Vec::with_capacity(32);
        for block in 0..32 {
            let frame = block * 48;
            let input = crate::analysis_stream::synthetic_inputs(frame)
                .map(|row| row.map(|sample| f64::from(sample) / 8_388_608.));
            let mut mixed = [0.; 192];
            reference_mixer
                .process_interleaved(input.as_flattened(), &mut mixed, 0)
                .unwrap();
            let mut sends = [0.; 96];
            for f in 0..48 {
                sends[f * 2..f * 2 + 2].copy_from_slice(&mixed[f * 4..f * 4 + 2]);
            }
            let safety =
                std::array::from_fn(|f| reference_mixer.output_safety_gain(frame + f as u64));
            let mut tb = [0.; 192];
            crate::brain_control::render_talkback_block(
                &crate::brain_control::TalkbackRender {
                    first_frame: frame,
                    deadline_frame: deadline,
                    monitors: &[0],
                    foh: true,
                    gain: 0.5,
                    safety: &safety,
                },
                &mut envelope,
                Some(&talkback),
                &mut tb,
                4,
            )
            .unwrap();
            reference_modules
                .process_interleaved_brain(
                    9,
                    frame,
                    input.as_flattened(),
                    &mixed,
                    4,
                    None,
                    Some(&tb),
                )
                .unwrap();
            for f in 0..48 {
                for bus in 2..4 {
                    mixed[f * 4 + bus] += tb[f * 4 + bus];
                }
            }
            expected.push((
                input,
                mixed,
                sends,
                *reference_modules.program_before_talkback(),
                reference_modules.output_interleaved().to_vec(),
            ));
        }
        assert!(
            expected
                .iter()
                .flat_map(|entry| &entry.4)
                .any(|sample| sample.abs() > 0.0001)
        );
        // Beyond the owner's 960-frame delay, actual local FX contributes to Main.
        assert!(expected[21..].iter().any(|entry| entry.2 != entry.3));
        modules
            .start(&directory.0, "kernel-take", Counter(1))
            .unwrap();
        let ready = (0..1000).any(|_| {
            modules.poll_lifecycle().unwrap();
            if modules.status(SHOW, 0).unwrap().recording.unwrap().state == "recording" {
                true
            } else {
                std::thread::sleep(Duration::from_millis(1));
                false
            }
        });
        assert!(ready, "recorder preparation timeout");
        mixer.prepare_render(48, 0).unwrap();
        let graph = std::thread::scope(|scope| {
            let source = mixer.take_source().unwrap();
            let module_source = modules.take_source().unwrap();
            let expected = &expected;
            scope
                .spawn(move || {
                    let mut graph = RenderGraph {
                        #[cfg(feature = "hardware-host")]
                        clock_routing: None,
                        mixer: source,
                        routing,
                        closure: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                        modules: Some(module_source),
                        aux: None,
                    };
                    let mut mixed = [0.; 192];
                    // The guard counts both allocation and destruction on this source
                    // worker. REC I/O remains on the owner's pre-existing worker.
                    crate::remote::assert_no_allocations(|| {
                        for (input, wanted, sends, monitor, pa) in expected {
                            graph
                                .render(
                                    9,
                                    input.as_flattened(),
                                    &mut mixed,
                                    None,
                                    Some(&talkback),
                                    None,
                                    RenderIntent {
                                        monitor: MonitorSource::Main,
                                        deadline,
                                        monitors: &[0],
                                        foh: true,
                                        talkback_gain: 0.5,
                                        remote_fx_aux: false,
                                    },
                                )
                                .unwrap();
                            assert_eq!(&mixed, wanted);
                            assert_eq!(&graph.routing.brain_send, sends);
                            assert_eq!(&graph.routing.brain_monitor, monitor);
                            assert_eq!(graph.modules.as_ref().unwrap().output_interleaved(), pa);
                            assert!(graph.mixer.busy());
                        }
                    });
                    graph
                })
                .join()
                .unwrap()
        });
        mixer.restore_source(graph.mixer).unwrap();
        modules.restore_source(graph.modules.unwrap()).unwrap();
        assert_eq!(mixer.frame(), 1536);
        mixer.finish_render().unwrap();
        modules.observe_after_render();
        modules.stop("kernel-take", Counter(1)).unwrap();
        let finalized = (0..1000).any(|_| {
            modules.poll_lifecycle().unwrap();
            if modules.status(SHOW, 100).unwrap().recording.unwrap().state == "finalized" {
                true
            } else {
                std::thread::sleep(Duration::from_millis(1));
                false
            }
        });
        assert!(finalized, "recorder finalization timeout");
        let status = modules.status(SHOW, 200).unwrap().recording.unwrap();
        assert_eq!(status.outcome, "complete");
        assert_eq!(status.accepted_frames.0, 1536);
        assert_eq!(status.written_frames.0, 1536);
        assert_eq!(status.dropped_frames.0, 0);
        let mut wavs: Vec<_> = std::fs::read_dir(directory.0.join("kernel-take"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "wav"))
            .collect();
        wavs.sort();
        assert_eq!(wavs.len(), 8);
        for (channel, path) in wavs.iter().enumerate() {
            let mut wav = hound::WavReader::open(path).unwrap();
            assert_eq!(wav.spec().bits_per_sample, 24);
            let actual: Vec<_> = wav.samples::<i32>().map(Result::unwrap).collect();
            let wanted: Vec<_> = expected
                .iter()
                .flat_map(|entry| entry.0.iter())
                .map(|row| (row[channel] * 8_388_608.).round() as i32)
                .collect();
            assert_eq!(actual, wanted);
        }
    }

    #[test]
    fn talkback_envelope_survives_quanta_and_fault_restarts_fade() {
        let topology = EngineTopology::legacy();
        let buses = topology.monitors + 2;
        let mut state = SourceRouting::new(&topology, None);
        let samples = [0.5; 48];
        let safety = [1.; 48];
        let mut expected = vec![0.; 48 * buses];
        let mut envelope = 0.;
        for block in 0..32 {
            if block == 16 {
                state.close_brain_audio();
                envelope = 0.;
                assert!(state.brain_talkback.iter().all(|v| *v == 0.));
            }
            let view = crate::brain_control::TalkbackRender {
                first_frame: block * 48,
                deadline_frame: Some(31 * 48 + 17),
                monitors: &[0],
                foh: false,
                gain: 0.5,
                safety: &safety,
            };
            let peaks = crate::brain_control::render_talkback_block(
                &view,
                &mut envelope,
                Some(&samples),
                &mut expected,
                buses,
            )
            .unwrap();
            assert_eq!(state.render_talkback(&view, Some(&samples), buses), peaks);
            assert_eq!(state.brain_talkback, expected);
        }
    }

    #[test]
    fn monitor_taps_precede_talkback_and_keep_unaligned_source_index() {
        let topology = EngineTopology::legacy();
        let buses = topology.monitors + 2;
        let mut state = SourceRouting::new(&topology, None);
        let mut output = vec![0.25; 48 * buses];
        let operator: Vec<_> = (0..96).map(|i| i as f64 / 128.).collect();
        state.capture_send(buses, &output);
        state.capture_monitor(
            MonitorSource::Pfl { input: 0 },
            47,
            buses,
            &output,
            &operator,
        );
        for f in 0..48 {
            let i = ((47 + f) % 48) * 2;
            assert_eq!(&state.brain_monitor[f * 2..f * 2 + 2], &operator[i..i + 2]);
        }
        state.capture_monitor(MonitorSource::Main, 47, buses, &output, &operator);
        state.brain_talkback.fill(0.5);
        state.mix_monitor_talkback(buses, &mut output);
        assert_eq!(state.brain_monitor, [0.25; 96]);
        assert_eq!(state.brain_send, [0.25; 96]);
        assert_eq!(state.meter_main, [0.25; 96]);
        for row in output.chunks_exact(buses) {
            assert_eq!(&row[..2], &[0.25; 2]);
            assert!(row[2..].iter().all(|v| *v == 0.75));
        }
    }
}

#[cfg(all(test, feature = "hardware-host"))]
mod return_routing_tests {
    use super::*;
    #[test]
    fn remote_returns_reach_selected_monitor_without_excitation_feedback_or_main() {
        let mut engine = crate::mixer_control::OfflineEngine::new(
            "11111111-1111-4111-8111-111111111111",
            crate::show::Counter(9),
            crate::show::Counter(0),
            0,
        )
        .unwrap();
        engine.enable_fx_aux().unwrap();
        for bus in 0..4 {
            engine
                .set_fx_returns(
                    bus,
                    &crate::fx_aux_wire::Returns {
                        main: 0.,
                        monitors: vec![if bus == 2 { 0.5 } else { 0. }, 0.],
                    },
                )
                .unwrap();
        }
        let routing = SourceRouting::new(engine.topology(), None);
        engine.prepare_render(48, 0).unwrap();
        let mut graph = RenderGraph {
            mixer: engine.take_source().unwrap(),
            routing,
            closure: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            clock_routing: None,
            modules: None,
            aux: None,
        };
        let wet: [f64; 384] = std::array::from_fn(|i| {
            if i % 8 == 4 {
                0.25
            } else if i % 8 == 5 {
                0.75
            } else {
                0.
            }
        });
        let mut output = [0.; 192];
        crate::remote::assert_no_allocations(|| {
            for _ in 0..7 {
                graph
                    .render(
                        9,
                        &[0.; 384],
                        &mut output,
                        Some(&wet),
                        None,
                        None,
                        RenderIntent {
                            monitor: MonitorSource::Monitor { index: 0 },
                            deadline: None,
                            monitors: &[],
                            foh: false,
                            talkback_gain: 0.,
                            remote_fx_aux: true,
                        },
                    )
                    .unwrap();
            }
        });
        for row in output.chunks_exact(4) {
            assert_eq!(row, &[0., 0., 0.25, 0.]);
        }
        assert_eq!(graph.routing.brain_send, [0.; 96]);
        assert!(
            graph
                .mixer
                .mixer
                .fx_aux_samples()
                .iter()
                .flatten()
                .all(|v| *v == 0.)
        );
        assert_eq!(
            graph.routing.brain_monitor,
            [0.25 * std::f64::consts::FRAC_1_SQRT_2; 96]
        );
        graph
            .closure
            .store(true, std::sync::atomic::Ordering::Release);
        crate::remote::assert_no_allocations(|| {
            assert!(
                graph
                    .render(
                        9,
                        &[0.; 384],
                        &mut output,
                        Some(&wet),
                        None,
                        None,
                        RenderIntent {
                            monitor: MonitorSource::Main,
                            deadline: None,
                            monitors: &[],
                            foh: false,
                            talkback_gain: 0.,
                            remote_fx_aux: true,
                        }
                    )
                    .is_err()
            );
        });
        assert_eq!(output, [0.; 192]);
    }
}
