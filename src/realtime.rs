//! Opt-in realtime scheduling for media worker threads.
//!
//! Off by default. A runner that is explicitly configured with a base priority
//! calls [`configure`] once, before it starts workers; each media thread then calls
//! [`enter`] as its first action. Priorities are graded: the source clock preempts
//! everything; network I/O (mostly blocked in epoll) sits just below it so packet
//! receipt is never delayed by FX/encoder computation, which runs lowest. The
//! opposite order measurably delayed Brain wet returns in GP-H1 T3. A failure (for example
//! no `RLIMIT_RTPRIO`) leaves that thread at normal priority and is counted, never
//! hidden: callers include [`report`] in their run evidence.
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

static BASE: AtomicI32 = AtomicI32::new(0);
static APPLIED: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);
static LAST_ERRNO: AtomicI32 = AtomicI32::new(0);
/// 0 = not attempted, 1 = locked, otherwise -errno.
static MEMORY_LOCK: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Source clock / device duplex owner.
    Source,
    /// Encoders, FX owners and media assembly workers.
    Worker,
    /// QUIC/network runtime threads (above workers, below the source clock).
    Network,
}
impl Role {
    fn offset(self) -> i32 {
        match self {
            Role::Source => 0,
            Role::Network => 2,
            Role::Worker => 5,
        }
    }
}

/// Valid bases leave room for the graded offsets inside SCHED_FIFO 1..=99.
pub fn validate(base: i32) -> Result<(), String> {
    if (11..=95).contains(&base) {
        Ok(())
    } else {
        Err("media realtime priority must be 11..=95".into())
    }
}
/// Select the base priority for subsequently started media threads.
pub fn configure(base: Option<i32>) -> Result<(), String> {
    if let Some(b) = base {
        validate(b)?;
    }
    BASE.store(base.unwrap_or(0), Ordering::Release);
    Ok(())
}
/// Lock current and future pages into RAM so realtime threads never wait on a
/// page fault behind filesystem I/O (observed on a 2 GiB Pi 5 while recording:
/// evicted code pages stalled the controller 10-20 ms). Needs RLIMIT_MEMLOCK.
/// The outcome is reported, never fatal.
pub fn lock_memory() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: plain syscall with constant flags.
        let code = if unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) } == 0 {
            1
        } else {
            -std::io::Error::last_os_error().raw_os_error().unwrap_or(1)
        };
        MEMORY_LOCK.store(code, Ordering::Release);
    }
}
/// The priority `role` would receive, or None when realtime is not configured.
pub fn priority(role: Role) -> Option<i32> {
    let base = BASE.load(Ordering::Acquire);
    (base > 0).then(|| base - role.offset())
}
/// Apply the configured policy to the calling thread. No-op when unconfigured.
pub fn enter(role: Role) {
    let Some(priority) = priority(role) else {
        return;
    };
    #[cfg(target_os = "linux")]
    {
        let parameter = libc::sched_param {
            sched_priority: priority,
        };
        // SAFETY: pid 0 addresses the calling thread; the parameter is a valid local.
        if unsafe { libc::sched_setscheduler(0, libc::SCHED_FIFO, &parameter) } == 0 {
            APPLIED.fetch_add(1, Ordering::Relaxed);
        } else {
            FAILED.fetch_add(1, Ordering::Relaxed);
            LAST_ERRNO.store(
                std::io::Error::last_os_error().raw_os_error().unwrap_or(-1),
                Ordering::Relaxed,
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = priority;
        FAILED.fetch_add(1, Ordering::Relaxed);
    }
}
/// Evidence for run reports.
pub fn report() -> serde_json::Value {
    let base = BASE.load(Ordering::Acquire);
    serde_json::json!({
        "base_priority": (base > 0).then_some(base),
        "threads_applied": APPLIED.load(Ordering::Relaxed),
        "threads_failed": FAILED.load(Ordering::Relaxed),
        "last_errno": match LAST_ERRNO.load(Ordering::Relaxed) { 0 => None, e => Some(e) },
        "memory_locked": match MEMORY_LOCK.load(Ordering::Acquire) {
            0 => serde_json::Value::Null,
            1 => true.into(),
            _ => false.into(),
        },
        // Linux compaction may migrate mlocked pages unless this is 0; realtime
        // threads then wait on page migration (seen in GP-H1 T3).
        "compact_unevictable_allowed": std::fs::read_to_string("/proc/sys/vm/compact_unevictable_allowed")
            .ok()
            .and_then(|v| v.trim().parse::<u8>().ok()),
        "memory_lock_errno": match MEMORY_LOCK.load(Ordering::Acquire) { e if e < 0 => Some(-e), _ => None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graded_priorities_validation_and_default_off() {
        assert_eq!(priority(Role::Source), None, "off unless configured");
        enter(Role::Worker); // no-op while unconfigured
        assert_eq!(report()["threads_failed"], 0);
        assert!(configure(Some(10)).is_err() && configure(Some(96)).is_err());
        configure(Some(70)).unwrap();
        assert_eq!(
            [Role::Source, Role::Worker, Role::Network].map(priority),
            [Some(70), Some(65), Some(68)]
        );
        // Without RLIMIT_RTPRIO (normal test environments) the failure is counted.
        // Counters are process-wide, so parallel tests may add to them.
        let count = |r: serde_json::Value| {
            r["threads_applied"].as_u64().unwrap() + r["threads_failed"].as_u64().unwrap()
        };
        let before = count(report());
        std::thread::spawn(|| enter(Role::Worker)).join().unwrap();
        assert!(count(report()) > before);
        configure(None).unwrap();
        assert_eq!(priority(Role::Network), None);
    }
}
