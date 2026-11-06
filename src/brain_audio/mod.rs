//! Brain duplex I/O and independent-clock bridges. Neither construction nor
//! normal tests activate physical audio; explicit device opening is feature gated.
pub mod bridge;
pub mod device;
pub mod host;

/// Touch prepared sample pages before starting the service clock. The opaque
/// read/write prevents a redundant zero-fill from being elided by optimization.
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
pub(crate) fn prefault_samples(samples: &mut [f64]) {
    for sample in samples.iter_mut().step_by(512) {
        *sample = std::hint::black_box(*sample);
    }
    if let Some(sample) = samples.last_mut() {
        *sample = std::hint::black_box(*sample);
    }
}
