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
