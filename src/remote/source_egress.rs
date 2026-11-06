//! Source samples cross a prepared four-block lane to the dedicated encoder.
//! Only the encoder owns packet allocation. Closing never joins on the source;
//! controller-side replacement/teardown joins and destroys the old worker.
use super::{MediaDescriptor, MediaRole, Result, encode_grouped};
use crate::transport::MediaDatagram;
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

const BLOCKS: usize = 4;
const PACKETS: usize = 256;
struct Block {
    frame: u64,
    analysis: Box<[f64]>,
    fx: Box<[f64]>,
}
#[derive(Default)]
struct Shared {
    closed: AtomicBool,
    #[cfg(test)]
    paused: AtomicBool,
    #[cfg(test)]
    pause_seen: AtomicBool,
    packet_drops: AtomicU64,
    encoded_packets: AtomicU64,
    drained_packets: AtomicU64,
    encode_refusals: AtomicU64,
    offered: AtomicU64,
    offer_refusals: AtomicU64,
    encoded: AtomicU64,
    discarded: AtomicU64,
    unconsumed_packets: AtomicU64,
}
#[derive(Default, Clone, Copy, serde::Serialize)]
pub(super) struct Counts {
    offered_blocks: u64,
    offer_refusals: u64,
    encoded_blocks: u64,
    encode_refusals: u64,
    packet_drops: u64,
    encoded_packets: u64,
    drained_packets: u64,
    discarded_blocks: u64,
    unconsumed_packets: u64,
}
impl Counts {
    pub fn add(&mut self, other: Self) {
        self.offer_refusals = self.offer_refusals.saturating_add(other.offer_refusals);
        self.offered_blocks = self.offered_blocks.saturating_add(other.offered_blocks);
        self.encoded_blocks = self.encoded_blocks.saturating_add(other.encoded_blocks);
        self.encode_refusals = self.encode_refusals.saturating_add(other.encode_refusals);
        self.packet_drops = self.packet_drops.saturating_add(other.packet_drops);
        self.encoded_packets = self.encoded_packets.saturating_add(other.encoded_packets);
        self.drained_packets = self.drained_packets.saturating_add(other.drained_packets);
        self.discarded_blocks = self.discarded_blocks.saturating_add(other.discarded_blocks);
        self.unconsumed_packets = self
            .unconsumed_packets
            .saturating_add(other.unconsumed_packets);
    }
}
/// Exclusive render-side endpoint. It carries no encoder thread or packet
/// consumer. Return this endpoint to the controller before destruction: its
/// retained sample buffers and ring storage must not be freed in a quantum.
pub(crate) struct SourceEgressInput {
    producer: Producer<Block>,
    returned: Consumer<Block>,
    spare: Option<Block>,
    shared: Arc<Shared>,
}
#[derive(Default)]
pub(super) struct SourceEgress {
    // Compatibility owners keep this here; independent production owners take
    // it once and return it with the rest of their retired source storage.
    input: Option<SourceEgressInput>,
    packets: Arc<Mutex<Option<Consumer<MediaDatagram>>>>,
    output_taken: bool,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    prepared_bytes: usize,
}
/// Sole network-side packet consumer. Its mutex only serializes packet draining
/// with controller shutdown; neither source rendering nor encoding acquires it.
/// Shutdown joins the encoder and drains/destroys the ring on the controller,
/// retaining exact accounting even when the network endpoint outlives its owner.
pub struct SourceEgressOutput {
    packets: Arc<Mutex<Option<Consumer<MediaDatagram>>>>,
    shared: Arc<Shared>,
}
impl SourceEgressOutput {
    pub(crate) fn poll(&mut self) -> Option<Vec<u8>> {
        poll_packet(&self.packets, &self.shared)
    }
}
fn poll_packet(
    packets: &Mutex<Option<Consumer<MediaDatagram>>>,
    shared: &Shared,
) -> Option<Vec<u8>> {
    let mut packets = packets.lock().unwrap_or_else(|error| error.into_inner());
    if shared.closed.load(Ordering::Acquire) {
        return None;
    }
    let packet = packets.as_mut()?.pop().ok()?;
    shared.drained_packets.fetch_add(1, Ordering::Relaxed);
    Some(packet.bytes().to_vec())
}
impl SourceEgress {
    fn prepared_bytes(analysis: usize, fx: usize) -> Result<usize> {
        let samples = analysis
            .checked_add(fx)
            .and_then(|n| n.checked_mul(BLOCKS * 8))
            .ok_or("source sample storage overflow")?;
        let queues = (2 * BLOCKS * std::mem::size_of::<Block>())
            .checked_add(PACKETS * std::mem::size_of::<MediaDatagram>())
            .ok_or("source queue storage overflow")?;
        samples
            .checked_add(queues)
            .ok_or_else(|| "source storage overflow".into())
    }

    pub fn prepare(descriptor: MediaDescriptor, analysis: usize, fx: usize) -> Result<Self> {
        if analysis > 48 * 256 || !matches!(fx, 0 | 96 | 384) || (fx == 0 && analysis == 0) {
            return Err("source egress dimensions".into());
        }
        let prepared_bytes = Self::prepared_bytes(analysis, fx)?;
        let (mut producer, mut input) = RingBuffer::<Block>::new(BLOCKS);
        let (mut recycle, mut returned) = RingBuffer::new(BLOCKS);
        for _ in 0..BLOCKS {
            let mut block = Block {
                frame: 0,
                analysis: vec![0.; analysis].into_boxed_slice(),
                fx: vec![0.; fx].into_boxed_slice(),
            };
            // Volatile stores ensure the compiler cannot eliminate page touching.
            for sample in block.analysis.iter_mut().chain(block.fx.iter_mut()) {
                // SAFETY: each reference is exclusively owned valid sample storage.
                unsafe {
                    std::ptr::write_volatile(sample, 0.);
                }
            }
            recycle
                .push(block)
                .map_err(|_| "source recycle preparation")?;
        }
        for _ in 0..BLOCKS {
            producer
                .push(returned.pop().map_err(|_| "source pool preparation")?)
                .map_err(|_| "source input preparation")?;
        }
        for _ in 0..BLOCKS {
            recycle
                .push(input.pop().map_err(|_| "source input preparation")?)
                .map_err(|_| "source pool preparation")?;
        }
        let (mut output, packets) = RingBuffer::<MediaDatagram>::new(PACKETS);
        // Touch the output ring too; no first-use packet pages on source execution.
        let mut packets = packets;
        for _ in 0..PACKETS {
            output
                .push(
                    MediaDatagram::copy_from(&[0; crate::transport::MAX_DATAGRAM])
                        .map_err(|_| "source packet preparation")?,
                )
                .map_err(|_| "source packet preparation")?;
        }
        for _ in 0..PACKETS {
            let _ = packets.pop();
        }
        let shared = Arc::new(Shared::default());
        let state = shared.clone();
        let worker = std::thread::Builder::new()
            .name(
                if fx == 0 {
                    "gp-analysis-encode"
                } else {
                    "gp-source-encode"
                }
                .into(),
            )
            .spawn(move || {
                while !state.closed.load(Ordering::Acquire) {
                    #[cfg(test)]
                    if state.paused.load(Ordering::Acquire) {
                        state.pause_seen.store(true, Ordering::Release);
                        std::thread::sleep(Duration::from_micros(100));
                        continue;
                    }
                    let Ok(block) = input.pop() else {
                        std::thread::sleep(Duration::from_micros(100));
                        continue;
                    };
                    let encoded = (|| -> Result<Vec<Vec<u8>>> {
                        let mut packets = Vec::new();
                        if descriptor.channels(MediaRole::Analysis) > 0 {
                            packets.extend(encode_grouped(
                                &descriptor,
                                MediaRole::Analysis,
                                block.frame,
                                &block.analysis,
                            )?);
                        }
                        if descriptor.channels(MediaRole::FxSend) > 0 {
                            packets.extend(encode_grouped(
                                &descriptor,
                                MediaRole::FxSend,
                                block.frame,
                                &block.fx,
                            )?);
                        }
                        Ok(packets)
                    })();
                    let success = encoded.is_ok();
                    if let Ok(packets) = encoded {
                        for packet in packets {
                            state.encoded_packets.fetch_add(1, Ordering::Relaxed);
                            if state.closed.load(Ordering::Acquire)
                                || MediaDatagram::copy_from(&packet)
                                    .map_or(true, |packet| output.push(packet).is_err())
                            {
                                state.packet_drops.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    // Four total blocks exist. The source can only return ownership
                    // by submitting one, so its recycle slot is always reserved.
                    if recycle.push(block).is_err() {
                        state.closed.store(true, Ordering::Release);
                        break;
                    }
                    if success {
                        state.encoded.fetch_add(1, Ordering::Release);
                    } else {
                        state.encode_refusals.fetch_add(1, Ordering::Release);
                    }
                }
                while input.pop().is_ok() {
                    state.discarded.fetch_add(1, Ordering::Relaxed);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            input: Some(SourceEgressInput {
                producer,
                returned,
                spare: None,
                shared: shared.clone(),
            }),
            packets: Arc::new(Mutex::new(Some(packets))),
            output_taken: false,
            shared,
            worker: Some(worker),
            prepared_bytes,
        })
    }
    /// Move the sole sample producer to the production source. Taking again
    /// refuses without creating a second writer or reopening a closed lane.
    pub fn take_input(&mut self) -> Option<SourceEgressInput> {
        self.input.take()
    }
    #[cfg(test)]
    pub fn offer(&mut self, frame: u64, analysis: &[f64], fx: &[f64]) -> bool {
        self.input
            .as_mut()
            .is_some_and(|input| input.offer(frame, analysis, fx))
    }
    /// Independent terminal invalidation, also closes an extracted endpoint.
    pub fn clear(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
    }
    /// Move packet draining to the authenticated network worker. The controller
    /// retains only shutdown access; it cannot also consume after this transfer.
    pub fn take_output(&mut self) -> Option<SourceEgressOutput> {
        if self.output_taken || self.worker.is_none() || self.shared.closed.load(Ordering::Acquire)
        {
            return None;
        }
        self.output_taken = true;
        Some(SourceEgressOutput {
            packets: self.packets.clone(),
            shared: self.shared.clone(),
        })
    }
    /// Compatibility controller-side conversion for callers without a proxy.
    pub fn poll(&mut self) -> Option<Vec<u8>> {
        if self.output_taken {
            return None;
        }
        poll_packet(&self.packets, &self.shared)
    }
    /// Controller-only shutdown; the source uses `clear` instead.
    pub fn finish(&mut self) {
        self.clear();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let packets = self
            .packets
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(mut packets) = packets {
            while packets.pop().is_ok() {
                self.shared
                    .unconsumed_packets
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    #[cfg(test)]
    pub fn pause(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Release);
        if paused {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !self.shared.pause_seen.load(Ordering::Acquire) {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_micros(100));
            }
        }
    }
    #[cfg(test)]
    pub fn wait_encoded(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while self.shared.encoded.load(Ordering::Acquire)
            + self.shared.encode_refusals.load(Ordering::Acquire)
            < self.shared.offered.load(Ordering::Acquire)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "encoder did not complete"
            );
            std::thread::sleep(Duration::from_micros(100));
        }
    }
    pub fn counts(&self) -> Counts {
        Counts {
            offer_refusals: self.shared.offer_refusals.load(Ordering::Acquire),
            offered_blocks: self.shared.offered.load(Ordering::Acquire),
            encoded_blocks: self.shared.encoded.load(Ordering::Acquire),
            encode_refusals: self.shared.encode_refusals.load(Ordering::Acquire),
            packet_drops: self.shared.packet_drops.load(Ordering::Acquire),
            encoded_packets: self.shared.encoded_packets.load(Ordering::Acquire),
            drained_packets: self.shared.drained_packets.load(Ordering::Acquire),
            discarded_blocks: self.shared.discarded.load(Ordering::Acquire),
            unconsumed_packets: self.shared.unconsumed_packets.load(Ordering::Acquire),
        }
    }
    pub fn report(&self, mut retired: Counts) -> serde_json::Value {
        retired.add(self.counts());
        serde_json::json!({
            "all_sessions": retired,
            "current_prepared_requested_bytes": self.prepared_bytes,
            "prepared_excludes": "rtrb shared and packet-shutdown mutex metadata, descriptor strings, encoder temporaries, thread stack, allocator overhead; not RSS",
            "closed": self.shared.closed.load(Ordering::Acquire),
        })
    }
}
impl SourceEgressInput {
    pub fn offer(&mut self, frame: u64, analysis: &[f64], fx: &[f64]) -> bool {
        if self.shared.closed.load(Ordering::Acquire) {
            return false;
        }
        if self.spare.is_none() {
            self.spare = self.returned.pop().ok();
        }
        let Some(slot) = self.spare.as_mut() else {
            self.shared.offer_refusals.fetch_add(1, Ordering::Relaxed);
            return false;
        };
        if slot.analysis.len() != analysis.len() || slot.fx.len() != fx.len() {
            self.shared.offer_refusals.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        slot.frame = frame;
        slot.analysis.copy_from_slice(analysis);
        slot.fx.copy_from_slice(fx);
        match self.producer.push(self.spare.take().unwrap()) {
            Ok(()) => {
                self.shared.offered.fetch_add(1, Ordering::Release);
                true
            }
            Err(rtrb::PushError::Full(block)) => {
                self.spare = Some(block);
                self.shared.offer_refusals.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }
    /// Terminal invalidation. No destructor, waiting, or queue drain on source.
    pub fn clear(&mut self) {
        self.shared.closed.store(true, Ordering::Release);
    }
}
impl Drop for SourceEgress {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::show::Counter;
    fn descriptor() -> MediaDescriptor {
        let ids = ["foh-left".into(), "foh-right".into()];
        let mut streams = super::super::grouped_streams(
            MediaRole::FxSend,
            &ids,
            48,
            0,
            1,
            crate::transport::MAX_DATAGRAM,
        )
        .unwrap();
        streams.extend(
            super::super::grouped_streams(
                MediaRole::WetReturn,
                &ids,
                48,
                768,
                2,
                crate::transport::MAX_DATAGRAM,
            )
            .unwrap(),
        );
        MediaDescriptor {
            session: Counter(1),
            source_epoch: Counter(2),
            capability_generation: Counter(3),
            map_generation: Counter(4),
            sample_rate: 48_000,
            streams,
        }
    }
    #[test]
    fn transferred_network_drain_and_controller_shutdown_keep_exact_accounting() {
        let mut controller = SourceEgress::prepare(descriptor(), 0, 96).unwrap();
        let mut source = controller.take_input().unwrap();
        let mut output = controller.take_output().unwrap();
        assert!(controller.take_output().is_none());
        let drained = Arc::new(std::sync::Barrier::new(2));
        let closed = Arc::new(std::sync::Barrier::new(2));
        let (network_drained, network_closed) = (drained.clone(), closed.clone());
        let network = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            for frame in 0..64 {
                let bytes = loop {
                    if let Some(bytes) = output.poll() {
                        break bytes;
                    }
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::yield_now();
                };
                let packet = crate::transport::Packet::parse(&bytes).unwrap();
                assert_eq!(packet.source_frame(), frame * 48);
                assert_eq!(packet.sample(0).unwrap(), 0.25);
            }
            network_drained.wait();
            network_closed.wait();
            assert!(output.poll().is_none());
        });
        // No controller poll or mailbox service participates in delivery.
        for frame in 0..64 {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                let mut offered = false;
                crate::remote::assert_no_allocations(|| {
                    offered = source.offer(frame * 48, &[], &[0.25; 96]);
                });
                if offered {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
        drained.wait();
        assert!(
            controller.poll().is_none(),
            "controller cannot compete with network"
        );
        // Close with a full output lane while the detached endpoint is alive.
        for frame in 64..64 + PACKETS as u64 + 4 {
            assert!(source.offer(frame * 48, &[], &[0.25; 96]));
            controller.wait_encoded();
        }
        crate::remote::assert_no_allocations(|| source.clear());
        controller.finish();
        closed.wait();
        network.join().unwrap();
        let counts = controller.counts();
        assert_eq!(counts.drained_packets, 64);
        assert_eq!(counts.unconsumed_packets, PACKETS as u64);
        assert_eq!(counts.packet_drops, 4);
        assert_eq!(
            counts.encoded_packets,
            counts.drained_packets + counts.unconsumed_packets + counts.packet_drops
        );
        assert!(!source.offer(0, &[], &[0.25; 96]));
    }

    #[test]
    fn transferred_source_retains_samples_under_encoder_stall() {
        let mut controller = SourceEgress::prepare(descriptor(), 0, 96).unwrap();
        controller.pause(true);
        let mut source = controller.take_input().unwrap();
        assert!(controller.take_input().is_none());
        // The controller neither services nor lends its encoder owner while the
        // source runs. Joining returns all source storage for controller disposal.
        let source = std::thread::spawn(move || {
            crate::remote::assert_no_allocations(|| {
                for n in 0..BLOCKS {
                    assert!(source.offer(n as u64 * 48, &[], &[n as f64 / 8.; 96]));
                }
                for _ in 0..32 {
                    assert!(!source.offer(192, &[], &[0.5; 96]));
                }
            });
            source
        })
        .join()
        .unwrap();
        controller.pause(false);
        controller.wait_encoded();
        for n in 0..BLOCKS {
            let bytes = controller.poll().unwrap();
            let packet = crate::transport::Packet::parse(&bytes).unwrap();
            assert_eq!(packet.source_frame(), n as u64 * 48);
            for index in 0..96 {
                assert_eq!(packet.sample(index).unwrap(), n as f64 / 8.);
            }
        }
        assert_eq!(controller.counts().offered_blocks, BLOCKS as u64);
        assert_eq!(controller.counts().encoded_blocks, BLOCKS as u64);
        controller.finish();
        drop(source);
    }

    #[test]
    fn controller_closure_reaches_transferred_source_with_full_input() {
        let mut controller = SourceEgress::prepare(descriptor(), 0, 96).unwrap();
        controller.pause(true);
        let mut source = controller.take_input().unwrap();
        let ready = Arc::new(std::sync::Barrier::new(2));
        let closed = Arc::new(std::sync::Barrier::new(2));
        let source_ready = ready.clone();
        let source_closed = closed.clone();
        let worker = std::thread::spawn(move || {
            crate::remote::assert_no_allocations(|| {
                for n in 0..BLOCKS {
                    assert!(source.offer(n as u64 * 48, &[], &[0.25; 96]));
                }
            });
            source_ready.wait();
            source_closed.wait();
            crate::remote::assert_no_allocations(|| {
                assert!(!source.offer(192, &[], &[0.25; 96]));
                source.clear();
                assert!(!source.offer(240, &[], &[0.25; 96]));
            });
            source
        });
        ready.wait();
        // No command slot or encoder progress is needed to close the source.
        controller.clear();
        closed.wait();
        let source = worker.join().unwrap();
        controller.finish();
        assert!(controller.poll().is_none());
        assert_eq!(controller.counts().discarded_blocks, BLOCKS as u64);
        assert_eq!(controller.counts().encoded_blocks, 0);
        drop(source);
    }

    #[test]
    fn source_encoder_saturation_closure_and_samples() {
        let mut lane = SourceEgress::prepare(descriptor(), 0, 96).unwrap();
        lane.shared.paused.store(true, Ordering::Release);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !lane.shared.pause_seen.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_micros(100));
        }
        let samples = [0.25; 96];
        crate::remote::assert_no_allocations(|| {
            for n in 0..4 {
                assert!(lane.offer(n * 48, &[], &samples));
            }
            assert!(!lane.offer(192, &[], &samples));
        });
        lane.shared.paused.store(false, Ordering::Release);
        lane.wait_encoded();
        for n in 0..4 {
            let bytes = lane.poll().unwrap();
            let packet = crate::transport::Packet::parse(&bytes).unwrap();
            assert_eq!(packet.source_frame(), n * 48);
            assert_eq!(packet.sample(0).unwrap(), 0.25);
        }
        for n in 4..304 {
            assert!(lane.offer(n * 48, &[], &samples));
            lane.wait_encoded();
        }
        assert_eq!(lane.counts().packet_drops, 44);
        crate::remote::assert_no_allocations(|| {
            lane.clear();
            assert!(!lane.offer(304 * 48, &[], &samples));
        });
        assert!(lane.poll().is_none());
        lane.finish();
        let counts = lane.counts();
        assert_eq!(counts.offered_blocks, 304);
        assert_eq!(counts.encoded_blocks, 304);
        assert_eq!(counts.unconsumed_packets, 256);
        assert_eq!(counts.discarded_blocks, 0);
        assert_eq!(counts.encoded_packets, 304);
        assert_eq!(counts.drained_packets, 4);
        assert_eq!(
            counts.encoded_packets,
            counts.drained_packets + counts.packet_drops + counts.unconsumed_packets
        );
    }
}
