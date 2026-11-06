//! GP-SPECTRUM:1/2. Selected raw pre-processing digital samples; no acoustic calibration.
//! Callback copies into a fixed two-window SPSC queue. FFT and publication run off-thread.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;
const N: usize = 4096;
pub const BINS: usize = 64;
// Top-bit tap tags keep configured playback slots independent of the legacy
// input wire index domain. Resource admission bounds the remaining address bits.
pub const OUTPUT: usize = 1usize << (usize::BITS - 2);
pub const MAIN: usize = 1usize << (usize::BITS - 1);
#[derive(Clone, Debug, Serialize)]
pub struct Source {
    pub id: String,
    pub label: String,
    pub tap: &'static str,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub contract: String,
    pub version: u32,
    pub show_id: String,
    pub epoch: u64,
    pub map: u64,
    pub query: u64,
    pub input: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub contract: &'static str,
    pub version: u32,
    pub supported: bool,
    pub show_id: String,
    pub epoch: u64,
    pub map: u64,
    pub query: u64,
    pub input: String,
    pub sample_rate: u32,
    pub tap: &'static str,
    pub units: &'static str,
    pub first_frame: u64,
    pub end_frame: u64,
    pub acquisition_age_ms: u64,
    pub valid: bool,
    pub bins: Vec<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fft_bins: Option<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<Source>>,
}
struct Window {
    samples: [f64; N],
    input: usize,
    generation: u64,
    first: u64,
    received: u64,
    map: u64,
}
#[derive(Clone)]
struct ResultWindow {
    input: usize,
    generation: u64,
    first: u64,
    received: u64,
    map: u64,
    bins: Vec<i32>,
    fft_bins: Vec<i32>,
}
/// Controller readback and worker lifetime; never transferred to the source.
pub struct Spectrum {
    selected: Arc<AtomicUsize>,
    generation: Arc<AtomicU64>,
    expires: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    latest: Arc<Mutex<Option<ResultWindow>>>,
    acquisition: Option<Box<Acquisition>>,
    rate: u32,
    worker: Option<std::thread::JoinHandle<()>>,
}
/// Fixed acquisition storage and atomic selection only. No FFT/readback lock or join.
pub struct Acquisition {
    selected: Arc<AtomicUsize>,
    generation: Arc<AtomicU64>,
    expires: Arc<AtomicU64>,
    sender: rtrb::Producer<Window>,
    samples: [f64; N],
    count: usize,
    first: u64,
    received: u64,
    input: usize,
    acquired_generation: u64,
    map: u64,
    next: u64,
    rate: u32,
}
/// Hann-windowed native positive FFT bins (DC through Nyquist).
/// 0 dBFS means a bin-centred sinusoid of peak amplitude 1; floor -96 dBFS.
pub fn analyze_fft(samples: &[f64; N]) -> Vec<i32> {
    let mut fft: Vec<_> = samples
        .iter()
        .enumerate()
        .map(|(i, x)| {
            [
                x * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / N as f64).cos()),
                0.,
            ]
        })
        .collect();
    crate::automix::analysis::fft(&mut fft);
    fft.iter()
        .take(N / 2 + 1)
        .enumerate()
        .map(|(k, v)| {
            let scale = if k == 0 || k == N / 2 { 2. } else { 4. };
            (20000.
                * (v[0].hypot(v[1]) * scale / N as f64)
                    .max(10f64.powf(-4.8))
                    .log10())
            .round()
            .clamp(-96000., 24000.) as i32
        })
        .collect()
}
pub fn aggregate(fft: &[i32], rate: u32) -> Vec<i32> {
    let mut peaks = vec![-96000; BINS];
    for (k, v) in fft.iter().enumerate().take(N / 2).skip(1) {
        let hz = k as f64 * rate as f64 / N as f64;
        if (20. ..=20000.).contains(&hz) {
            let bin = (((hz / 20.).log10() / 3.) * BINS as f64).floor() as usize;
            peaks[bin.min(BINS - 1)] = peaks[bin.min(BINS - 1)].max(*v);
        }
    }
    peaks
}
pub fn analyze(samples: &[f64; N], rate: u32) -> Vec<i32> {
    aggregate(&analyze_fft(samples), rate)
}

impl Spectrum {
    pub fn new(rate: u32) -> Self {
        let (sender, mut receiver) = rtrb::RingBuffer::<Window>::new(2);
        let selected = Arc::new(AtomicUsize::new(usize::MAX));
        let stop = Arc::new(AtomicBool::new(false));
        let latest = Arc::new(Mutex::new(None));
        let (out, halt) = (latest.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !halt.load(Ordering::Relaxed) {
                let mut newest = None;
                // Fixed queue capacity: bounded drain; obsolete windows discarded.
                for _ in 0..2 {
                    if let Ok(w) = receiver.pop() {
                        newest = Some(w);
                    }
                }
                if let Some(w) = newest {
                    let fft_bins = analyze_fft(&w.samples);
                    let bins = aggregate(&fft_bins, rate);
                    *out.lock().unwrap() = Some(ResultWindow {
                        input: w.input,
                        generation: w.generation,
                        fft_bins,
                        first: w.first,
                        received: w.received,
                        map: w.map,
                        bins,
                    });
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        let generation = Arc::new(AtomicU64::new(0));
        let expires = Arc::new(AtomicU64::new(0));
        let acquisition = Box::new(Acquisition {
            selected: selected.clone(),
            generation: generation.clone(),
            expires: expires.clone(),
            sender,
            samples: [0.; N],
            count: 0,
            first: 0,
            received: 0,
            input: usize::MAX,
            acquired_generation: 0,
            map: 0,
            next: 0,
            rate,
        });
        Self {
            selected,
            generation,
            expires,
            stop,
            latest,
            acquisition: Some(acquisition),
            rate,
            worker: Some(worker),
        }
    }
    pub fn take_acquisition(&mut self) -> Option<Box<Acquisition>> {
        self.acquisition.take()
    }
    pub fn offer(&mut self, pcm: &[f64], channels: usize, frame: u64, received: u64, map: u64) {
        if let Some(a) = &mut self.acquisition {
            a.offer(pcm, channels, frame, received, map);
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn offer_tap(
        &mut self,
        pcm: &[f64],
        channels: usize,
        frame: u64,
        received: u64,
        map: u64,
        base: usize,
    ) {
        if let Some(a) = &mut self.acquisition {
            a.offer_tap(pcm, channels, frame, received, map, base);
        }
    }
}
impl Acquisition {
    /// No locks, allocation, syscalls or FFT. Input is interleaved source PCM.
    pub fn offer(&mut self, pcm: &[f64], channels: usize, frame: u64, received: u64, map: u64) {
        self.offer_tap(pcm, channels, frame, received, map, 0);
    }
    #[allow(clippy::too_many_arguments)] // Explicit buffer shape, tap address and source timestamp.
    pub fn offer_tap(
        &mut self,
        pcm: &[f64],
        channels: usize,
        frame: u64,
        received: u64,
        map: u64,
        base: usize,
    ) {
        let selected = self.selected.load(Ordering::Relaxed);
        if selected < base || selected >= base.saturating_add(channels) {
            return;
        }
        let generation = self.generation.load(Ordering::Relaxed);
        if received > self.expires.load(Ordering::Relaxed) {
            self.count = 0;
            return;
        }
        let input = selected - base;
        if input >= channels || channels == 0 {
            return;
        }
        if generation != self.acquired_generation
            || selected != self.input
            || map != self.map
            || (self.count != 0 && frame != self.first + self.count as u64)
        {
            self.count = 0;
            self.next = frame;
            self.input = selected;
            self.acquired_generation = generation;
            self.map = map;
        }
        for (i, chunk) in pcm.chunks_exact(channels).enumerate() {
            let at = frame + i as u64;
            if at < self.next {
                continue;
            }
            if self.count == 0 {
                self.first = at;
                self.received = received;
            }
            if !chunk[input].is_finite() || chunk[input].abs() > 16. {
                self.count = 0;
                self.next = at + self.rate as u64 / 10;
                continue;
            }
            self.samples[self.count] = chunk[input];
            self.count += 1;
            if self.count == N {
                let _ = self.sender.push(Window {
                    samples: self.samples,
                    input: selected,
                    generation,
                    first: self.first,
                    received: self.received,
                    map,
                });
                self.count = 0;
                self.next = self.first + self.rate as u64 / 10;
            }
        }
    }
}
impl Spectrum {
    /// Source-epoch fence, called off render during owner recovery.
    pub fn reset(&mut self) {
        self.selected.store(usize::MAX, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.expires.store(0, Ordering::Relaxed);
        *self.latest.lock().unwrap() = None;
    }
    pub fn snapshot(&self, r: &Request, index: usize, now: u64) -> Snapshot {
        self.expires
            .store(now.saturating_add(1000), Ordering::Relaxed);
        if self.selected.swap(index, Ordering::Relaxed) != index {
            self.generation.fetch_add(1, Ordering::Relaxed);
            *self.latest.lock().unwrap() = None;
        }
        let generation = self.generation.load(Ordering::Relaxed);
        let w = self
            .latest
            .lock()
            .unwrap()
            .clone()
            .filter(|w| w.input == index && w.map == r.map && w.generation == generation);
        let valid = w
            .as_ref()
            .is_some_and(|w| now.saturating_sub(w.received) <= 350);
        Snapshot {
            contract: "GP-SPECTRUM",
            version: r.version,
            supported: true,
            show_id: r.show_id.clone(),
            epoch: r.epoch,
            map: r.map,
            query: r.query,
            input: r.input.clone(),
            sample_rate: self.rate,
            tap: if index >= MAIN {
                "main-pre-pa"
            } else if index >= OUTPUT {
                "physical-output-post-patch"
            } else {
                "raw-pre-processing"
            },
            units: "millidBFS-peak-Hann-max",
            first_frame: w.as_ref().map_or(0, |w| w.first),
            end_frame: w.as_ref().map_or(0, |w| w.first + N as u64),
            acquisition_age_ms: w.as_ref().map_or(0, |w| now.saturating_sub(w.received)),
            valid,
            fft_bins: (r.version == 2).then(|| {
                if valid {
                    w.as_ref().unwrap().fft_bins.clone()
                } else {
                    vec![]
                }
            }),
            sources: None,
            bins: if valid { w.unwrap().bins } else { vec![] },
        }
    }
}
impl Drop for Spectrum {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detached_acquisition_survives_stalled_readback_and_reset() {
        let mut s = Spectrum::new(48000);
        let mut acquisition = s.take_acquisition().unwrap();
        let r = Request {
            contract: "GP-SPECTRUM".into(),
            version: 2,
            show_id: "test".into(),
            epoch: 1,
            map: 1,
            query: 1,
            input: "input-01".into(),
        };
        assert!(!s.snapshot(&r, 0, 1000).valid);
        let acquisition = std::thread::spawn(move || {
            for b in 0..1024 {
                acquisition.offer(&[0.5; 48], 1, b * 48, 1000, 1);
            }
            acquisition
        })
        .join()
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !s.snapshot(&r, 0, 1100).valid {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        s.reset();
        assert!(!s.snapshot(&r, 0, 1100).valid);
        // Controller teardown joins its worker; it never requires the source
        // acquisition to return, nor can later source delivery join anything.
        drop(s);
        let mut acquisition = acquisition;
        acquisition.offer(&[0.5; 48], 1, 49152, 1100, 1);
    }
    #[test]
    fn stale_selection_and_bounded_handoff() {
        let mut s = Spectrum::new(48000);
        let r = Request {
            contract: "GP-SPECTRUM".into(),
            version: 1,
            show_id: "test".into(),
            epoch: 1,
            map: 1,
            query: 1,
            input: "input-01".into(),
        };
        assert!(!s.snapshot(&r, 0, 1000).valid);
        for i in 0..86 {
            s.offer(&[0.5; 48], 1, i * 48, 1000, 1);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !s.snapshot(&r, 0, 1100).valid {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!s.snapshot(&r, 0, 1351).valid);
        assert!(!s.snapshot(&r, 1, 1100).valid);
        assert!(
            !s.snapshot(&r, 0, 1100).valid,
            "A-B-A must not recover old observation"
        );
        s.reset();
        assert!(!s.snapshot(&r, 0, 1100).valid);
        let mut r = r;
        r.map = 2;
        assert!(!s.snapshot(&r, 0, 1100).valid);
    }
    #[test]
    fn tone_and_silence() {
        for k in [8, 85, 853] {
            let x = std::array::from_fn(|i| {
                0.5 * (std::f64::consts::TAU * k as f64 * i as f64 / N as f64).sin()
            });
            let b = analyze(&x, 48000);
            assert!((b.iter().max().unwrap() + 6021).abs() < 3);
            let peak = b.iter().position(|v| Some(v) == b.iter().max()).unwrap();
            let hz = k as f64 * 48000. / N as f64;
            assert_eq!(peak, ((hz / 20.).log10() / 3. * 64.).floor() as usize);
        }
        assert!(analyze(&[0.; N], 48000).iter().all(|v| *v == -96000));
    }
}
