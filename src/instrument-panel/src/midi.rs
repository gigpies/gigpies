//! Read-only MIDI 1.0 acquisition shared by both consoles.
//!
//! The reader opens one explicitly named ALSA raw-MIDI endpoint read-only and
//! delivers *complete* channel messages stamped with an acquisition generation.
//! It never writes to the device, enumerates presets or infers bindings: profile
//! decoding and authority checks stay in each console's existing adapter.
//! Every reopen increments the generation so delayed input from an earlier
//! connection can never act on a new one.

use std::{
    fs::File,
    io::{ErrorKind, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread::JoinHandle,
    time::Duration,
};

/// One complete MIDI channel-voice message (1..=3 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    bytes: [u8; 3],
    len: u8,
}
impl Message {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// Byte-stream parser: running status, interleaved realtime bytes and
/// discarded SysEx/system-common data. Orphan data bytes are dropped.
#[derive(Default, Debug)]
pub struct Parser {
    status: Option<u8>,
    data: [u8; 2],
    have: usize,
    sysex: bool,
}
fn data_len(status: u8) -> usize {
    match status & 0xf0 {
        0xc0 | 0xd0 => 1,
        _ => 2,
    }
}
impl Parser {
    pub fn push(&mut self, byte: u8) -> Option<Message> {
        if byte >= 0xf8 {
            return None; // realtime: never interrupts a message in progress
        }
        if byte >= 0xf0 {
            // System common/SysEx cancels running status; content is not delivered.
            self.status = None;
            self.have = 0;
            self.sysex = byte == 0xf0;
            return None;
        }
        if byte >= 0x80 {
            self.status = Some(byte);
            self.have = 0;
            self.sysex = false;
            return None;
        }
        if self.sysex {
            return None;
        }
        let status = self.status?;
        self.data[self.have] = byte;
        self.have += 1;
        if self.have < data_len(status) {
            return None;
        }
        self.have = 0;
        let len = 1 + data_len(status);
        Some(Message {
            bytes: [
                status,
                self.data[0],
                if len == 3 { self.data[1] } else { 0 },
            ],
            len: len as u8,
        })
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Resolve a stable `/dev/snd/by-id` controller name to its raw-MIDI node.
/// Only device 0 of the matching card is accepted; no other port is guessed.
pub fn resolve_by_id(snd: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.len() > 128 || id.contains('/') || id.starts_with('.') {
        return Err("controller identity".into());
    }
    let link = std::fs::read_link(snd.join("by-id").join(id))
        .map_err(|_| format!("controller {id} not present"))?;
    let name = link
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("controller link")?;
    let card: u32 = name
        .strip_prefix("controlC")
        .and_then(|n| n.parse().ok())
        .ok_or("controller link is not a card control node")?;
    let node = snd.join(format!("midiC{card}D0"));
    if !node.exists() {
        return Err(format!("controller {id} has no raw MIDI port"));
    }
    Ok(node)
}

/// Delivered acquisition event. `Lost` is sent once per failed connection so the
/// console clears held state; the next `Message` carries a newer generation.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Message {
        endpoint: Arc<str>,
        message: Message,
        generation: u64,
    },
    Lost {
        endpoint: Arc<str>,
        generation: u64,
    },
}

/// Bounded read-only acquisition worker. A full queue drops input and counts it;
/// it never blocks acquisition or replays stale messages later.
pub struct Reader {
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}
pub const QUEUE: usize = 256;
impl Reader {
    /// `open` is re-evaluated on every reconnect so a replugged controller with a
    /// new card number is found again by its stable identity.
    pub fn spawn(
        endpoint: &str,
        open: impl Fn() -> Result<PathBuf, String> + Send + 'static,
    ) -> Result<(Self, Receiver<Event>), String> {
        let endpoint: Arc<str> = endpoint.into();
        let (tx, rx) = sync_channel(QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let (s, d) = (stop.clone(), dropped.clone());
        let thread = std::thread::Builder::new()
            .name("midi-acquisition".into())
            .spawn(move || run(endpoint, open, tx, s, d))
            .map_err(|e| e.to_string())?;
        Ok((
            Self {
                stop,
                dropped,
                thread: Some(thread),
            },
            rx,
        ))
    }
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
const POLL: Duration = Duration::from_millis(2);
const RETRY: Duration = Duration::from_millis(500);
fn open_read_only(path: &Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
}
fn send(tx: &SyncSender<Event>, dropped: &AtomicU64, event: Event) -> bool {
    match tx.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            dropped.fetch_add(1, Ordering::Relaxed);
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}
fn sleep_unless(stop: &AtomicBool, total: Duration) {
    let mut left = total;
    while !left.is_zero() && !stop.load(Ordering::Acquire) {
        let step = left.min(Duration::from_millis(20));
        std::thread::sleep(step);
        left -= step;
    }
}
fn run(
    endpoint: Arc<str>,
    open: impl Fn() -> Result<PathBuf, String>,
    tx: SyncSender<Event>,
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
) {
    let mut generation = 0u64;
    let mut buffer = [0u8; 256];
    while !stop.load(Ordering::Acquire) {
        let Ok(mut file) = open().and_then(|p| open_read_only(&p).map_err(|e| e.to_string()))
        else {
            sleep_unless(&stop, RETRY);
            continue;
        };
        generation += 1;
        let mut parser = Parser::default();
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            match file.read(&mut buffer) {
                Ok(0) => break, // writer closed / device gone
                Ok(n) => {
                    for &b in &buffer[..n] {
                        if let Some(message) = parser.push(b)
                            && !send(
                                &tx,
                                &dropped,
                                Event::Message {
                                    endpoint: endpoint.clone(),
                                    message,
                                    generation,
                                },
                            )
                        {
                            return;
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(POLL),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        // `Lost` must not be silently dropped: retry until delivered or stopped.
        let mut lost = Event::Lost {
            endpoint: endpoint.clone(),
            generation,
        };
        loop {
            match tx.try_send(lost) {
                Ok(()) => break,
                Err(TrySendError::Disconnected(_)) => return,
                Err(TrySendError::Full(e)) => {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    lost = e;
                    std::thread::sleep(POLL);
                }
            }
        }
        sleep_unless(&stop, RETRY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut p = Parser::default();
        bytes
            .iter()
            .filter_map(|b| p.push(*b))
            .map(|m| m.bytes().to_vec())
            .collect()
    }
    #[test]
    fn running_status_realtime_sysex_and_orphans() {
        assert_eq!(
            parse(&[0x40, 0x99, 36, 127, 37, 0, 0xf8, 0xb0, 1, 0xfe, 65]),
            vec![vec![0x99, 36, 127], vec![0x99, 37, 0], vec![0xb0, 1, 65]]
        );
        // SysEx content and the bytes after it without new status are discarded.
        assert_eq!(
            parse(&[0x90, 60, 0xf0, 0x00, 0x20, 0x6b, 0xf7, 61, 100, 0xc2, 5, 6]),
            vec![vec![0xc2, 5], vec![0xc2, 6]]
        );
        // A status byte interrupting an incomplete message discards the fragment.
        assert_eq!(parse(&[0x90, 60, 0x80, 60, 0]), vec![vec![0x80, 60, 0]]);
    }
    #[test]
    fn by_id_resolution_requires_card_link_and_port() {
        let root = std::env::temp_dir().join(format!("panel-midi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("by-id")).unwrap();
        std::os::unix::fs::symlink("../controlC3", root.join("by-id/usb-Test-00")).unwrap();
        std::os::unix::fs::symlink("../pcmC1D0p", root.join("by-id/usb-Pcm-00")).unwrap();
        assert!(resolve_by_id(&root, "usb-Test-00").is_err(), "no port yet");
        std::fs::write(root.join("midiC3D0"), b"").unwrap();
        assert_eq!(
            resolve_by_id(&root, "usb-Test-00").unwrap(),
            root.join("midiC3D0")
        );
        assert!(resolve_by_id(&root, "usb-Pcm-00").is_err());
        assert!(resolve_by_id(&root, "../x").is_err());
        assert!(resolve_by_id(&root, "absent").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn fifo_reader_stamps_generation_and_reports_loss_then_reconnects() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("panel-fifo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fifo = root.join("midi");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        // A FIFO reports EOF while no writer exists, so the test writer opens
        // read-write (non-blocking on Linux) before the acquisition opens.
        let open_writer = || {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&fifo)
                .unwrap()
        };
        let mut writer = open_writer();
        let path = fifo.clone();
        let (reader, rx) = Reader::spawn("test-pad", move || Ok(path.clone())).unwrap();
        let wait = Duration::from_secs(5);
        writer.write_all(&[0x99, 36, 127, 37, 0]).unwrap();
        let first = rx.recv_timeout(wait).unwrap();
        let Event::Message {
            message,
            generation,
            ..
        } = first
        else {
            panic!("{first:?}")
        };
        assert_eq!((message.bytes(), generation), (&[0x99, 36, 127][..], 1));
        assert!(matches!(
            rx.recv_timeout(wait).unwrap(),
            Event::Message { generation: 1, .. }
        ));
        drop(writer);
        assert_eq!(
            rx.recv_timeout(wait).unwrap(),
            Event::Lost {
                endpoint: "test-pad".into(),
                generation: 1
            }
        );
        let mut writer = open_writer();
        writer.write_all(&[0xb0, 24, 65]).unwrap();
        // Reconnect attempts made before the writer returned may report further
        // losses; the delivered message must carry a newer generation.
        let reconnected = loop {
            match rx.recv_timeout(wait).unwrap() {
                Event::Lost { generation, .. } => assert!(generation > 1),
                Event::Message {
                    message,
                    generation,
                    ..
                } => break (message, generation),
            }
        };
        assert_eq!(reconnected.0.bytes(), &[0xb0, 24, 65]);
        assert!(reconnected.1 > 1);
        assert_eq!(reader.dropped(), 0);
        drop(reader);
        drop(writer);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
