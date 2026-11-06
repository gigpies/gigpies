//! Controller reconstruction of an autonomous synthetic source epoch. The old
//! owner closes independently; reconstruction never replays grants or rearms.
use super::*;

pub struct SourceClockRecovery {
    pub previous: SourceClockReport,
    /// A failed reconstruction retains the previous report and leaves the
    /// source closed. Callers must not interpret failure as a running old epoch.
    pub replacement: Result<SourceClockStatus>,
}

impl HostAuthority {
    /// Replace a running synthetic source on its existing worker, preserving the
    /// original absolute run deadline. Physical devices require a separately
    /// validated reopen. The replacement progresses while outputs stay disarmed.
    pub fn recover_source_clock(
        &mut self,
        status: &SourceClockStatus,
        epoch: Counter,
        frame: u64,
        now_ms: u64,
    ) -> Result<SourceClockRecovery> {
        if !self
            .source_clock_status
            .as_ref()
            .is_some_and(|active| active.same_owner(status))
        {
            return Err("source clock status identity".into());
        }
        if epoch.0 <= self.provider.source_epoch() || !frame.is_multiple_of(48) {
            return Err("fresh epoch and aligned source frame required".into());
        }
        let entered = std::time::Instant::now();
        let (period, deadline, blocks) = status.recovery_budget()?;
        if frame.checked_add(blocks * 48).is_none() {
            return Err("source recovery frame exhausted".into());
        }
        let previous = self.finish_source_clock(status)?;
        let replacement = (|| {
            self.clear_media_history();
            // Device intents and read latches belong to the old Stagebox epoch.
            if let Some(change) = self.device_pending.take() {
                self.finish_device(
                    &change,
                    "failed_device",
                    Some("source epoch replaced".into()),
                );
            }
            self.device_reads.clear();
            self.device_messages.clear();
            self.device_observation = None;
            self.device_observed_ms = None;
            self.device_owner = None;
            self.provider.recover_source(epoch, frame)?;
            self.observed_source_epoch = epoch.0;
            // Charge reconstruction time against the original run, including
            // preparation and queued installation. No recovery extends a run.
            let (_, _, remaining) = status.recovery_budget()?;
            let resumed_now = now_ms.saturating_add(entered.elapsed().as_millis() as u64);
            let mut state =
                source_clock::SourceClock::prepare(&self.provider, resumed_now, period, remaining)?;
            state.retain_run_deadline(deadline);
            self.install_source_clock(state)
        })();
        Ok(SourceClockRecovery {
            previous,
            replacement,
        })
    }
}
