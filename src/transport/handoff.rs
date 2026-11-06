use super::Error;
use rtrb::{Consumer, Producer, RingBuffer};

/// Independent preallocated queues for the network and recorder workers.
/// Copy blocks have no destructor; a rejected push never frees heap memory.
/// The host owns routing/DSP and must report recorder overflow as a recording gap.
pub struct CaptureFanout<T: Copy> {
    network: Producer<T>,
    recorder: Producer<T>,
    pub network_drops: u64,
    pub recorder_drops: u64,
}
pub type CaptureEndpoints<T> = (CaptureFanout<T>, Consumer<T>, Consumer<T>);
impl<T: Copy> CaptureFanout<T> {
    pub fn new(
        network_capacity: usize,
        recorder_capacity: usize,
    ) -> Result<CaptureEndpoints<T>, Error> {
        if !(1..=8192).contains(&network_capacity) || !(1..=8192).contains(&recorder_capacity) {
            return Err(Error::Capacity);
        }
        let (network, n) = RingBuffer::new(network_capacity);
        let (recorder, r) = RingBuffer::new(recorder_capacity);
        Ok((
            Self {
                network,
                recorder,
                network_drops: 0,
                recorder_drops: 0,
            },
            n,
            r,
        ))
    }
    /// Call once per block; never retry/wait here. Worker stalls are independent.
    pub fn offer(&mut self, block: T) {
        if self.network.push(block).is_err() {
            self.network_drops = self.network_drops.saturating_add(1);
        }
        if self.recorder.push(block).is_err() {
            self.recorder_drops = self.recorder_drops.saturating_add(1);
        }
    }
}

/// Inline datagram storage for SPSC ingress. Copy ownership means dequeue,
/// rejection and queue overflow cannot free a payload on the receiving thread.
#[derive(Clone, Copy, Debug)]
pub struct MediaDatagram {
    len: u16,
    bytes: [u8; super::MAX_DATAGRAM],
}
impl MediaDatagram {
    pub fn copy_from(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() || bytes.len() > super::MAX_DATAGRAM {
            return Err(Error::Size);
        }
        let mut packet = Self {
            len: bytes.len() as u16,
            bytes: [0; super::MAX_DATAGRAM],
        };
        packet.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(packet)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }
}
