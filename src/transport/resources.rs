//! Checked payload budget for the proposed isolated 48-frame execution profile.
//! This is setup arithmetic, not a claim that the legacy runtime uses these pools
//! or that payload bytes include allocator, DSP, TLS or process overhead.
use super::{Error, MAX_DATAGRAM};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioPayloadBudget {
    pub fx_handoffs: usize,
    pub packet_windows: usize,
    pub duplex_handoffs: usize,
    pub asrc_rings: usize,
    pub recorder: usize,
    pub analysis: usize,
    pub total: usize,
}
impl AudioPayloadBudget {
    /// `groups` counts every independently allocated 32-packet window across
    /// both processes, including wet playout; it is not just streams on one link.
    /// Validate each descriptor's <=64 groups separately before calling.
    pub fn checked(inputs: usize, fx_channels: usize, groups: usize) -> Result<Self, Error> {
        if inputs == 0 || !(1..=256).contains(&fx_channels) || groups == 0 {
            return Err(Error::Capacity);
        }
        let product = |a: usize, b: usize| a.checked_mul(b).ok_or(Error::Capacity);
        let fx_handoffs = product(4 * 4 * 48 * 8, fx_channels)?;
        let packet_windows = product(32 * MAX_DATAGRAM, groups)?;
        // Both ends of each crossing allocate a handoff: Brain capture ->
        // network -> Stagebox, and Stagebox monitor -> network -> Brain.
        let duplex_handoffs = 2 * 4 * 48 * 3 * 8;
        let asrc_rings = 4096 * 3 * 8;
        let recorder = product(256 * 48 * 8, inputs)?;
        if recorder > 64 * 1024 * 1024 {
            return Err(Error::Capacity);
        }
        let analysis = product(32 * 48 * 8, inputs)?;
        let total = [
            fx_handoffs,
            packet_windows,
            duplex_handoffs,
            asrc_rings,
            recorder,
            analysis,
        ]
        .into_iter()
        .try_fold(0usize, |a, b| a.checked_add(b).ok_or(Error::Capacity))?;
        Ok(Self {
            fx_handoffs,
            packet_windows,
            duplex_handoffs,
            asrc_rings,
            recorder,
            analysis,
            total,
        })
    }
}

/// Rust-owned prepared storage, excluding allocator bookkeeping/padding outside
/// the requested layouts. This is not process RSS or an owner-library estimate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PreparedStorage {
    pub inline_bytes: usize,
    pub heap_bytes: usize,
    pub total_bytes: usize,
}
impl PreparedStorage {
    pub fn checked(inline_bytes: usize, heap_bytes: usize) -> Result<Self, Error> {
        Ok(Self {
            inline_bytes,
            heap_bytes,
            total_bytes: inline_bytes
                .checked_add(heap_bytes)
                .ok_or(Error::Capacity)?,
        })
    }
    /// Compose separately owned allocations. Do not add an inline child twice
    /// when its containing object's layout has already been counted.
    pub fn checked_add(self, other: Self) -> Result<Self, Error> {
        Self::checked(
            self.inline_bytes
                .checked_add(other.inline_bytes)
                .ok_or(Error::Capacity)?,
            self.heap_bytes
                .checked_add(other.heap_bytes)
                .ok_or(Error::Capacity)?,
        )
    }
}
