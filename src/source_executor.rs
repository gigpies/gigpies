//! Exclusive source owners and globally ordered owned packets. The controller
//! may wait for a return; the source reserves return capacity before entering
//! a command and returns its storage without destruction. Optional source-clock
//! progression runs independently of controller calls and occupied return lanes.
use std::{
    any::Any,
    marker::PhantomData,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const COMMANDS: usize = 16;
const COMPLETIONS: usize = 128;
const RETIREMENTS: usize = 16;
const OWNERS: usize = 2;
/// Global queue payload layouts, counted once for both source-owner handles.
#[cfg(all(target_os = "linux", feature = "hardware-host"))]
pub(crate) fn prepared_queue_storage() -> serde_json::Value {
    serde_json::json!({"commands":COMMANDS,"completions":COMPLETIONS,"retirements":RETIREMENTS,
        "queue_payload_bytes":(COMMANDS + RETIREMENTS) * std::mem::size_of::<Box<dyn Packet>>(),
        "completion_payload_bytes":std::mem::size_of::<()>(),
        "outstanding_controller_packets":1,
        "excludes":"ring metadata, pointed-to commands, owner graph and thread stack"})
}
type Owner = Box<dyn Any + Send>;
type Slots = [Option<Owner>; OWNERS];
struct Owners {
    slots: Slots,
    progress: Option<ProgressEntry>,
    pending: Option<Box<dyn Packet>>,
}
trait Progress: Send {
    fn advance(&mut self, slots: &mut Slots, now: Instant) -> Duration;
    #[cfg(any(feature = "hardware-host", test))]
    fn into_state(self: Box<Self>) -> Box<dyn Any + Send>;
}
struct ProgressEntry {
    driver: Box<dyn Progress>,
    next: Instant,
}
#[cfg(any(feature = "hardware-host", test))]
struct OwnedProgress<T, U, P> {
    first: usize,
    second: usize,
    state: P,
    advance: fn(&mut T, &mut U, &mut P, Instant) -> Duration,
}
#[cfg(any(feature = "hardware-host", test))]
impl<T: Send + 'static, U: Send + 'static, P: Send + 'static> Progress for OwnedProgress<T, U, P> {
    fn advance(&mut self, slots: &mut Slots, now: Instant) -> Duration {
        let (left, right) = slots.split_at_mut(1);
        let (first, second) = if self.first < self.second {
            (&mut left[0], &mut right[0])
        } else {
            (&mut right[0], &mut left[0])
        };
        // Quiesced reconstruction may temporarily remove either owner. Never
        // call the driver with a placeholder or migrate it to another thread.
        let (Some(first), Some(second)) = (first.as_mut(), second.as_mut()) else {
            return Duration::from_millis(1);
        };
        (self.advance)(
            first
                .downcast_mut::<T>()
                .expect("progress first owner type"),
            second
                .downcast_mut::<U>()
                .expect("progress second owner type"),
            &mut self.state,
            now,
        )
    }
    #[cfg(any(feature = "hardware-host", test))]
    fn into_state(self: Box<Self>) -> Box<dyn Any + Send> {
        Box::new(self.state)
    }
}
#[cfg(any(feature = "hardware-host", test))]
struct ProgressTransfer {
    progress: Option<ProgressEntry>,
    install: bool,
    accepted: bool,
}
#[cfg(any(feature = "hardware-host", test))]
impl Packet for ProgressTransfer {
    fn apply(&mut self, owners: &mut Owners) -> bool {
        if self.install && owners.progress.is_some() {
            return true;
        }
        std::mem::swap(&mut self.progress, &mut owners.progress);
        self.accepted = true;
        true
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}
trait Packet: Send {
    fn completion_thread(&self) -> Option<&thread::Thread> {
        None
    }
    fn apply(&mut self, owners: &mut Owners) -> bool;
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;
}
// The notification is prepared and retired by the controller. Audio invokes
// only the wrapped packet; the executor wakes the waiter after publishing both
// return lanes, outside the guarded render/commit section.
struct NotifiedPacket {
    packet: Box<dyn Packet>,
    waiter: thread::Thread,
}
impl NotifiedPacket {
    fn prepare(packet: Box<dyn Packet>) -> Box<dyn Packet> {
        Box::new(Self {
            packet,
            waiter: thread::current(),
        })
    }
}
impl Packet for NotifiedPacket {
    fn completion_thread(&self) -> Option<&thread::Thread> {
        Some(&self.waiter)
    }
    fn apply(&mut self, owners: &mut Owners) -> bool {
        self.packet.apply(owners)
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self.packet.into_any()
    }
}
struct Operation<T, P> {
    owner: usize,
    packet: P,
    apply: fn(&mut T, &mut P),
}
impl<T: Send + 'static, P: Send + 'static> Packet for Operation<T, P> {
    fn apply(&mut self, owners: &mut Owners) -> bool {
        let owner = owners.slots[self.owner]
            .as_mut()
            .expect("source owner installed")
            .downcast_mut::<T>()
            .expect("source owner type");
        (self.apply)(owner, &mut self.packet);
        true
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}
#[cfg(any(feature = "hardware-host", test))]
struct ContinuedOperation<T, P> {
    owner: usize,
    packet: P,
    advance: fn(&mut T, &mut P) -> bool,
}
#[cfg(any(feature = "hardware-host", test))]
impl<T: Send + 'static, P: Send + 'static> Packet for ContinuedOperation<T, P> {
    fn apply(&mut self, owners: &mut Owners) -> bool {
        let owner = owners.slots[self.owner]
            .as_mut()
            .expect("source owner installed")
            .downcast_mut::<T>()
            .expect("source owner type");
        (self.advance)(owner, &mut self.packet)
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}
#[cfg(any(feature = "hardware-host", test))]
struct PairedOperation<T, U, P> {
    first: usize,
    second: usize,
    packet: P,
    advance: fn(&mut T, &mut U, &mut P) -> bool,
}
#[cfg(any(feature = "hardware-host", test))]
impl<T: Send + 'static, U: Send + 'static, P: Send + 'static> Packet for PairedOperation<T, U, P> {
    fn apply(&mut self, owners: &mut Owners) -> bool {
        let (left, right) = owners.slots.split_at_mut(1);
        let (first, second) = if self.first < self.second {
            (&mut left[0], &mut right[0])
        } else {
            (&mut right[0], &mut left[0])
        };
        (self.advance)(
            first
                .as_mut()
                .expect("paired first owner")
                .downcast_mut::<T>()
                .expect("paired first type"),
            second
                .as_mut()
                .expect("paired second owner")
                .downcast_mut::<U>()
                .expect("paired second type"),
            &mut self.packet,
        )
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}
struct Transfer {
    index: usize,
    owner: Option<Owner>,
}
impl Packet for Transfer {
    fn apply(&mut self, owners: &mut Owners) -> bool {
        std::mem::swap(&mut self.owner, &mut owners.slots[self.index]);
        true
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}
struct Client {
    commands: rtrb::Producer<Box<dyn Packet>>,
    completed: rtrb::Consumer<()>,
    retired: rtrb::Consumer<Box<dyn Packet>>,
    worker: Option<thread::JoinHandle<Owners>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    occupied: [bool; OWNERS],
}
impl Client {
    fn exchange(&mut self, packet: Box<dyn Packet>) -> Result<Box<dyn Packet>, String> {
        // One synchronous controller transaction at a time reserves its return
        // before publication. The 16 command slots also bound retained storage.
        if self.worker.as_ref().is_none_or(|w| w.is_finished()) {
            return Err("source executor stopped".into());
        }
        self.commands
            .push(NotifiedPacket::prepare(packet))
            .map_err(|_| "source command capacity".to_string())?;
        self.worker.as_ref().unwrap().thread().unpark();
        loop {
            if self.completed.pop().is_ok() {
                return self
                    .retired
                    .pop()
                    .map_err(|_| "source retirement missing".into());
            }
            if self.worker.as_ref().unwrap().is_finished() {
                return Err("source executor stopped before completion".into());
            }
            // Only the non-realtime controller waits. No locks are shared with
            // the source thread and closure signals remain independent atomics.
            // Completion unpark tokens survive publication before this wait.
            // Timeout only detects a panicked/stopped worker; it is not a media
            // deadline or an added delay on successful completion.
            thread::park_timeout(Duration::from_millis(100));
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            // Returning owners makes their final destruction controller work.
            let _ = worker.join();
        }
    }
}
pub(crate) struct SourceExecutor<T> {
    client: Arc<Mutex<Client>>,
    index: usize,
    active: bool,
    marker: PhantomData<T>,
}
impl<T: Send + 'static> SourceExecutor<T> {
    pub fn new(name: &str, owner: T) -> Result<Self, String> {
        let (commands, mut incoming) = rtrb::RingBuffer::<Box<dyn Packet>>::new(COMMANDS);
        let (mut completed, returns) = rtrb::RingBuffer::new(COMPLETIONS);
        let (mut retired, retirements) = rtrb::RingBuffer::<Box<dyn Packet>>::new(RETIREMENTS);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let source_stop = stop.clone();
        let mut owners = Owners {
            slots: [Some(Box::new(owner)), None],
            progress: None,
            pending: None,
        };
        let worker = thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                while !source_stop.load(std::sync::atomic::Ordering::Acquire) {
                    // One bounded progression step precedes command work, even
                    // if either return lane is full or the controller is stalled.
                    let now = Instant::now();
                    if let Some(progress) = &mut owners.progress
                        && now >= progress.next
                    {
                        #[cfg(test)]
                        let delay = {
                            let mut delay = Duration::ZERO;
                            crate::remote::assert_no_allocations(|| {
                                delay = progress.driver.advance(&mut owners.slots, now);
                            });
                            delay
                        };
                        #[cfg(not(test))]
                        let delay = progress.driver.advance(&mut owners.slots, now);
                        let after = Instant::now();
                        progress.next = after.checked_add(delay).unwrap_or(after);
                    }
                    // Reserve return capacity BEFORE removing a command or swapping
                    // any owner. An occupied return lane cannot lose retirement.
                    if owners.pending.is_none()
                        && !completed.is_full()
                        && !retired.is_full()
                        && let Ok(packet) = incoming.pop()
                    {
                        // This sole retained head owns the reserved return slots
                        // until completion; later commands cannot overtake it.
                        owners.pending = Some(packet);
                    }
                    if let Some(mut packet) = owners.pending.take() {
                        #[cfg(test)]
                        let done = {
                            let mut done = false;
                            crate::remote::assert_no_allocations(|| {
                                done = packet.apply(&mut owners);
                            });
                            done
                        };
                        #[cfg(not(test))]
                        let done = packet.apply(&mut owners);
                        if done {
                            let waiter = packet.completion_thread().cloned();
                            assert!(retired.push(packet).is_ok());
                            assert!(completed.push(()).is_ok());
                            if let Some(waiter) = waiter {
                                waiter.unpark();
                            }
                            continue;
                        }
                        owners.pending = Some(packet);
                    }
                    let delay =
                        owners
                            .progress
                            .as_ref()
                            .map_or(Duration::from_millis(1), |progress| {
                                progress
                                    .next
                                    .saturating_duration_since(Instant::now())
                                    .min(Duration::from_millis(1))
                            });
                    if !delay.is_zero() {
                        thread::park_timeout(delay);
                    }
                }
                owners
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client: Arc::new(Mutex::new(Client {
                commands,
                completed: returns,
                retired: retirements,
                worker: Some(worker),
                stop,
                occupied: [true, false],
            })),
            index: 0,
            active: true,
            marker: PhantomData,
        })
    }
    /// Transfer a prepared progression driver to this worker. Both owners must
    /// already occupy this executor's distinct slots. The callback must perform
    /// bounded work without allocations, destruction, locks or I/O.
    #[cfg(any(feature = "hardware-host", test))]
    pub fn start_progress<U: Send + 'static, P: Send + 'static>(
        &self,
        sibling: &SourceExecutor<U>,
        state: P,
        advance: fn(&mut T, &mut U, &mut P, Instant) -> Duration,
    ) -> Result<(), String> {
        if !self.active
            || !sibling.active
            || self.index == sibling.index
            || !Arc::ptr_eq(&self.client, &sibling.client)
        {
            return Err("source progress owners".into());
        }
        let packet = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?
            .exchange(Box::new(ProgressTransfer {
                progress: Some(ProgressEntry {
                    driver: Box::new(OwnedProgress {
                        first: self.index,
                        second: sibling.index,
                        state,
                        advance,
                    }),
                    next: Instant::now(),
                }),
                install: true,
                accepted: false,
            }))?;
        let packet = packet
            .into_any()
            .downcast::<ProgressTransfer>()
            .map_err(|_| "source progress transfer type")?;
        if !packet.accepted {
            return Err("source progress already installed".into());
        }
        Ok(())
    }
    /// Stop progression in global command order and return its prepared storage
    /// for controller-side finalization/destruction. Return capacity is reserved
    /// before the source removes the driver.
    #[cfg(any(feature = "hardware-host", test))]
    pub fn stop_progress<P: Send + 'static>(&self) -> Result<P, String> {
        let packet = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?
            .exchange(Box::new(ProgressTransfer {
                progress: None,
                install: false,
                accepted: false,
            }))?;
        let packet = packet
            .into_any()
            .downcast::<ProgressTransfer>()
            .map_err(|_| "source progress transfer type")?;
        let progress = packet.progress.ok_or("source progress absent")?;
        Ok(*progress
            .driver
            .into_state()
            .downcast::<P>()
            .map_err(|_| "source progress state type")?)
    }
    pub fn sibling<U: Send + 'static>(&self, owner: U) -> Result<SourceExecutor<U>, String> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?;
        let index = client
            .occupied
            .iter()
            .position(|v| !v)
            .ok_or("source owner capacity")?;
        let packet = client.exchange(Box::new(Transfer {
            index,
            owner: Some(Box::new(owner)),
        }))?;
        let packet = packet
            .into_any()
            .downcast::<Transfer>()
            .map_err(|_| "source transfer type")?;
        assert!(packet.owner.is_none());
        client.occupied[index] = true;
        Ok(SourceExecutor {
            client: self.client.clone(),
            index,
            active: true,
            marker: PhantomData,
        })
    }
    pub fn transact<P: Send + 'static>(
        &self,
        packet: P,
        apply: fn(&mut T, &mut P),
    ) -> Result<P, String> {
        if !self.active {
            return Err("source owner suspended".into());
        }
        let packet = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?
            .exchange(Box::new(Operation {
                owner: self.index,
                packet,
                apply,
            }))?;
        Ok(packet
            .into_any()
            .downcast::<Operation<T, P>>()
            .map_err(|_| "source packet type")?
            .packet)
    }
    /// Retain one globally ordered packet across source steps. `false` keeps
    /// its reserved returns and storage; source progression continues before the
    /// next attempt. The callback must eventually finish or observe closure.
    #[cfg(any(feature = "hardware-host", test))]
    pub fn transact_until<P: Send + 'static>(
        &self,
        packet: P,
        advance: fn(&mut T, &mut P) -> bool,
    ) -> Result<P, String> {
        if !self.active {
            return Err("source owner suspended".into());
        }
        let packet = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?
            .exchange(Box::new(ContinuedOperation {
                owner: self.index,
                packet,
                advance,
            }))?;
        Ok(packet
            .into_any()
            .downcast::<ContinuedOperation<T, P>>()
            .map_err(|_| "source packet type")?
            .packet)
    }
    /// One retained transaction across both owners on this worker. Return
    /// capacity is reserved before either owner can be mutated.
    #[cfg(any(feature = "hardware-host", test))]
    pub fn transact_pair_until<U: Send + 'static, P: Send + 'static>(
        &self,
        other: &SourceExecutor<U>,
        packet: P,
        advance: fn(&mut T, &mut U, &mut P) -> bool,
    ) -> Result<P, String> {
        if !self.active
            || !other.active
            || self.index == other.index
            || !Arc::ptr_eq(&self.client, &other.client)
        {
            return Err("source paired owner identity/state".into());
        }
        let packet = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?
            .exchange(Box::new(PairedOperation {
                first: self.index,
                second: other.index,
                packet,
                advance,
            }))?;
        Ok(packet
            .into_any()
            .downcast::<PairedOperation<T, U, P>>()
            .map_err(|_| "source paired packet type")?
            .packet)
    }
    /// Exercise production progression with a genuinely occupied return lane.
    /// The closure runs on the controller while its mutex stays held; it may
    /// inspect independent atomic status, but must not transact on this worker.
    #[cfg(test)]
    pub fn with_saturated_returns(&self, completion_lane: bool, test: impl FnOnce()) {
        fn wait(mut ready: impl FnMut() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !ready() {
                assert!(Instant::now() < deadline, "source saturation deadline");
                thread::park_timeout(Duration::from_millis(100));
            }
        }
        assert!(self.active, "source saturation owner suspended");
        let mut client = self.client.lock().expect("source controller poisoned");
        assert!(client.completed.is_empty());
        assert!(client.retired.is_empty());
        assert_eq!(client.commands.slots(), COMMANDS);
        let capacity = if completion_lane {
            COMPLETIONS
        } else {
            RETIREMENTS
        };
        // Retain returned allocations on the controller throughout the stall.
        // This permits filling all numeric slots independently of retirements.
        let mut storage = Vec::with_capacity(capacity);
        for index in 0..capacity {
            assert!(
                client
                    .commands
                    .push(NotifiedPacket::prepare(Box::new(Operation {
                        owner: self.index,
                        packet: (),
                        apply: |_: &mut T, _: &mut ()| {},
                    })))
                    .is_ok()
            );
            client.worker.as_ref().unwrap().thread().unpark();
            wait(|| client.completed.slots() == index + 1);
            if completion_lane {
                storage.push(client.retired.pop().expect("source retirement"));
            }
        }
        assert_eq!(client.completed.slots(), capacity);
        assert_eq!(
            client.retired.slots(),
            if completion_lane { 0 } else { RETIREMENTS }
        );
        // Restore queue accounting even if an assertion in the caller fails.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(test));
        for _ in 0..capacity {
            client.completed.pop().expect("source completion");
            if !completion_lane {
                storage.push(client.retired.pop().expect("source retirement"));
            }
        }
        assert!(client.completed.is_empty());
        assert!(client.retired.is_empty());
        client.worker.as_ref().unwrap().thread().unpark();
        drop(client);
        drop(storage);
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }
    #[cfg(test)]
    pub fn finish(mut self) -> Result<T, String> {
        let owner = self.take_owner()?;
        Ok(*owner.downcast::<T>().map_err(|_| "source owner type")?)
    }
    /// Retire a quiesced owner for controller-only lifecycle reconstruction.
    /// Keep its worker/slot reserved so sibling media cannot migrate threads.
    pub fn suspend(&mut self) -> Result<T, String> {
        if !self.active {
            return Err("source owner already suspended".into());
        }
        let mut client = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?;
        let packet = client.exchange(Box::new(Transfer {
            index: self.index,
            owner: None,
        }))?;
        let mut packet = packet
            .into_any()
            .downcast::<Transfer>()
            .map_err(|_| "source transfer type")?;
        self.active = false;
        Ok(*packet
            .owner
            .take()
            .ok_or("source owner absent")?
            .downcast::<T>()
            .map_err(|_| "source owner type")?)
    }
    pub fn resume(&mut self, owner: T) -> Result<(), String> {
        if self.active {
            return Err("source owner already installed".into());
        }
        let packet = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?
            .exchange(Box::new(Transfer {
                index: self.index,
                owner: Some(Box::new(owner)),
            }))?;
        let packet = packet
            .into_any()
            .downcast::<Transfer>()
            .map_err(|_| "source transfer type")?;
        assert!(packet.owner.is_none());
        self.active = true;
        Ok(())
    }
    #[cfg(test)]
    fn take_owner(&mut self) -> Result<Owner, String> {
        let mut client = self
            .client
            .lock()
            .map_err(|_| "source controller poisoned")?;
        let packet = client.exchange(Box::new(Transfer {
            index: self.index,
            owner: None,
        }))?;
        let packet = packet
            .into_any()
            .downcast::<Transfer>()
            .map_err(|_| "source transfer type")?;
        client.occupied[self.index] = false;
        self.active = false;
        packet.owner.ok_or_else(|| "source owner absent".into())
    }
}
impl<T> Drop for SourceExecutor<T> {
    fn drop(&mut self) {
        if self.active {
            // Drop cannot use the typed API; the transfer still returns native
            // and allocating storage intact to this controller thread.
            if let Ok(mut client) = self.client.lock() {
                let _ = client.exchange(Box::new(Transfer {
                    index: self.index,
                    owner: None,
                }));
                client.occupied[self.index] = false;
            }
        } else if let Ok(mut client) = self.client.lock() {
            client.occupied[self.index] = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(Instant::now() < deadline, "source test deadline");
            thread::yield_now();
        }
    }

    #[test]
    fn waiting_for_future_source_completion_does_not_busy_spin() {
        fn cpu_us() -> u64 {
            let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
            assert_eq!(
                unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut usage) },
                0
            );
            ((usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) * 1_000_000
                + usage.ru_utime.tv_usec
                + usage.ru_stime.tv_usec) as u64
        }
        let owner = SourceExecutor::new("source-notification", 0u64).unwrap();
        let sibling = owner.sibling(()).unwrap();
        owner
            .start_progress(&sibling, (), |steps, _, _, _| {
                *steps += 1;
                Duration::from_millis(1)
            })
            .unwrap();
        let before_cpu = cpu_us();
        let before = Instant::now();
        owner
            .transact_until(None, |steps, first| {
                let first = *first.get_or_insert(*steps);
                *steps >= first + 50
            })
            .unwrap();
        let cpu = cpu_us() - before_cpu;
        assert!(before.elapsed() >= Duration::from_millis(50));
        assert!(
            cpu < 20_000,
            "controller spent {cpu} CPU us waiting for source"
        );
        owner.stop_progress::<()>().unwrap();
    }
    #[test]
    fn paired_future_packet_keeps_owner_order_and_rejects_other_workers() {
        let owner = SourceExecutor::new("paired-source", 0u64).unwrap();
        let sibling = owner.sibling((0u64, false)).unwrap();
        let unrelated = SourceExecutor::new("other-source", 0u64).unwrap();
        assert!(
            owner
                .transact_pair_until(&unrelated, (), |_, _, _| true)
                .is_err()
        );
        owner
            .start_progress(&sibling, (), |first, second, _, _| {
                *first += 1;
                second.0 += 1;
                assert_eq!(*first, second.0);
                Duration::from_micros(50)
            })
            .unwrap();
        let observed = sibling
            .transact_pair_until(&owner, None, |second, first, result| {
                if *first < 10 {
                    return false;
                }
                assert_eq!(*first, second.0);
                second.1 = true;
                *result = Some(*first);
                true
            })
            .unwrap();
        owner.stop_progress::<()>().unwrap();
        assert!(observed.unwrap() >= 10);
        assert!(
            sibling
                .transact(false, |state, out| *out = state.1)
                .unwrap()
        );
    }

    fn progress_survives_full_returns(completion_lane: bool) {
        let steps = Arc::new(AtomicUsize::new(0));
        let changes = Arc::new(AtomicUsize::new(0));
        let owner = SourceExecutor::new("source-progress-pressure", changes.clone()).unwrap();
        let sibling = owner.sibling(steps.clone()).unwrap();
        owner
            .start_progress(&sibling, (), |_, steps, _, _| {
                steps.fetch_add(1, Ordering::Release);
                Duration::ZERO
            })
            .unwrap();
        // Hold the controller mutex and withhold returns. The source must keep
        // stepping, while an extra mutation remains behind reserved capacity.
        let mut client = owner.client.lock().unwrap();
        let capacity = if completion_lane {
            COMPLETIONS
        } else {
            RETIREMENTS
        };
        for index in 0..capacity {
            client
                .commands
                .push(Box::new(Operation {
                    owner: owner.index,
                    packet: (),
                    apply: |changes: &mut Arc<AtomicUsize>, _: &mut ()| {
                        changes.fetch_add(1, Ordering::Release);
                    },
                }))
                .ok()
                .unwrap();
            client.worker.as_ref().unwrap().thread().unpark();
            if completion_lane {
                wait_until(|| !client.retired.is_empty());
                drop(client.retired.pop().ok().unwrap());
            } else {
                wait_until(|| client.retired.slots() == index + 1);
            }
        }
        wait_until(|| client.completed.slots() == capacity);
        client
            .commands
            .push(Box::new(Operation {
                owner: owner.index,
                packet: (),
                apply: |changes: &mut Arc<AtomicUsize>, _: &mut ()| {
                    changes.fetch_add(1, Ordering::Release);
                },
            }))
            .ok()
            .unwrap();
        let before = steps.load(Ordering::Acquire);
        wait_until(|| steps.load(Ordering::Acquire) >= before + 32);
        assert_eq!(changes.load(Ordering::Acquire), capacity);
        for _ in 0..capacity {
            client.completed.pop().unwrap();
            if !completion_lane {
                drop(client.retired.pop().ok().unwrap());
            }
        }
        client.worker.as_ref().unwrap().thread().unpark();
        wait_until(|| !client.completed.is_empty());
        client.completed.pop().unwrap();
        drop(client.retired.pop().ok().unwrap());
        assert_eq!(changes.load(Ordering::Acquire), capacity + 1);
        drop(client);
        owner.stop_progress::<()>().unwrap();
        owner.finish().unwrap();
        sibling.finish().unwrap();
    }

    #[test]
    fn progression_survives_controller_stall_and_full_retirements() {
        progress_survives_full_returns(false);
    }

    #[test]
    fn progression_survives_controller_stall_and_full_completions() {
        progress_survives_full_returns(true);
    }

    #[test]
    fn saturation_fixture_restores_accounting_after_controller_stall() {
        let steps = Arc::new(AtomicUsize::new(0));
        let owner = SourceExecutor::new("source-saturation-fixture", steps.clone()).unwrap();
        let sibling = owner.sibling(()).unwrap();
        owner
            .start_progress(&sibling, (), |steps, _, _, _| {
                steps.fetch_add(1, Ordering::Release);
                Duration::ZERO
            })
            .unwrap();
        for completion_lane in [false, true] {
            owner.with_saturated_returns(completion_lane, || {
                let before = steps.load(Ordering::Acquire);
                wait_until(|| steps.load(Ordering::Acquire) >= before + 32);
            });
            assert!(owner.transact(false, |_, packet| *packet = true).unwrap());
        }
        owner.stop_progress::<()>().unwrap();
    }

    #[test]
    fn retained_command_finishes_after_independent_source_steps() {
        let owner = SourceExecutor::new("source-progress-pending", 0usize).unwrap();
        let sibling = owner.sibling(0usize).unwrap();
        owner
            .start_progress(&sibling, (), |first, second, _, _| {
                *first += 1;
                *second += 1;
                Duration::from_micros(100)
            })
            .unwrap();
        let packet = owner
            .transact_until((None, 0), |steps, packet| {
                let first = *packet.0.get_or_insert(*steps);
                packet.1 += 1;
                *steps >= first + 8
            })
            .unwrap();
        assert!(packet.1 > 1);
        owner.stop_progress::<()>().unwrap();
        assert_eq!(owner.finish().unwrap(), sibling.finish().unwrap());
    }

    #[test]
    fn progression_storage_stops_and_destroys_on_controller() {
        struct Storage(Arc<Mutex<Option<thread::ThreadId>>>);
        impl Drop for Storage {
            fn drop(&mut self) {
                *self.0.lock().unwrap() = Some(thread::current().id());
            }
        }
        let destroyed = Arc::new(Mutex::new(None));
        let owner = SourceExecutor::new("source-progress-retirement", ()).unwrap();
        let sibling = owner.sibling(()).unwrap();
        owner
            .start_progress(&sibling, Storage(destroyed.clone()), |_, _, _, _| {
                Duration::from_millis(1)
            })
            .unwrap();
        let storage = owner.stop_progress::<Storage>().unwrap();
        assert!(destroyed.lock().unwrap().is_none());
        drop(storage);
        assert_eq!(*destroyed.lock().unwrap(), Some(thread::current().id()));
    }

    #[test]
    fn shutdown_returns_unfinished_packet_without_source_destruction() {
        struct Storage {
            attempted: Arc<AtomicUsize>,
            destroyed: Arc<Mutex<Option<thread::ThreadId>>>,
        }
        impl Drop for Storage {
            fn drop(&mut self) {
                *self.destroyed.lock().unwrap() = Some(thread::current().id());
            }
        }
        let attempted = Arc::new(AtomicUsize::new(0));
        let destroyed = Arc::new(Mutex::new(None));
        let owner = SourceExecutor::new("source-pending-shutdown", ()).unwrap();
        let mut client = owner.client.lock().unwrap();
        client
            .commands
            .push(Box::new(ContinuedOperation {
                owner: owner.index,
                packet: Storage {
                    attempted: attempted.clone(),
                    destroyed: destroyed.clone(),
                },
                advance: |_: &mut (), packet: &mut Storage| {
                    packet.attempted.fetch_add(1, Ordering::Release);
                    false
                },
            }))
            .ok()
            .unwrap();
        client.worker.as_ref().unwrap().thread().unpark();
        wait_until(|| attempted.load(Ordering::Acquire) > 0);
        client.stop.store(true, Ordering::Release);
        client.worker.as_ref().unwrap().thread().unpark();
        wait_until(|| client.worker.as_ref().unwrap().is_finished());
        assert!(destroyed.lock().unwrap().is_none());
        drop(client);
        drop(owner);
        assert_eq!(*destroyed.lock().unwrap(), Some(thread::current().id()));
    }

    #[test]
    fn siblings_share_source_and_retire_on_controller() {
        struct Storage(Arc<Mutex<Option<thread::ThreadId>>>);
        impl Drop for Storage {
            fn drop(&mut self) {
                *self.0.lock().unwrap() = Some(thread::current().id());
            }
        }
        let destroyed = Arc::new(Mutex::new(None));
        let owner =
            SourceExecutor::new("source-owner-test", Some(Storage(destroyed.clone()))).unwrap();
        let sibling = owner.sibling(0usize).unwrap();
        let source_thread = owner
            .transact(None, |_, p| *p = Some(thread::current().id()))
            .unwrap();
        let sibling_thread = sibling
            .transact(None, |_, p| *p = Some(thread::current().id()))
            .unwrap();
        assert_eq!(source_thread, sibling_thread);
        assert_ne!(source_thread, Some(thread::current().id()));
        let retired = owner.transact(None, std::mem::swap).unwrap();
        assert!(destroyed.lock().unwrap().is_none());
        drop(retired);
        assert_eq!(*destroyed.lock().unwrap(), Some(thread::current().id()));
        assert!(owner.finish().unwrap().is_none());
        assert_eq!(sibling.finish().unwrap(), 0);
    }

    #[test]
    fn full_retirement_lane_reserves_before_any_further_mutation() {
        let changes = Arc::new(AtomicUsize::new(0));
        let owner = SourceExecutor::new("source-pressure-test", changes.clone()).unwrap();
        let mut client = owner.client.lock().unwrap();
        // Fill retained returns without consuming them, then leave one additional
        // prepared command queued. No source-side destructor or lost completion.
        for _ in 0..RETIREMENTS {
            client
                .commands
                .push(Box::new(Operation {
                    owner: owner.index,
                    packet: (),
                    apply: |state: &mut Arc<AtomicUsize>, _: &mut ()| {
                        state.fetch_add(1, Ordering::Release);
                    },
                }))
                .ok()
                .unwrap();
        }
        client.worker.as_ref().unwrap().thread().unpark();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while client.retired.slots() != RETIREMENTS {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        client
            .commands
            .push(Box::new(Operation {
                owner: owner.index,
                packet: (),
                apply: |state: &mut Arc<AtomicUsize>, _: &mut ()| {
                    state.fetch_add(1, Ordering::Release);
                },
            }))
            .ok()
            .unwrap();
        client.worker.as_ref().unwrap().thread().unpark();
        // The capacity invariant is deterministic: all return slots are held.
        assert_eq!(client.retired.slots(), RETIREMENTS);
        assert_eq!(changes.load(Ordering::Acquire), RETIREMENTS);
        for _ in 0..RETIREMENTS {
            while client.completed.pop().is_err() {
                thread::yield_now();
            }
            drop(client.retired.pop().ok().unwrap());
        }
        client.worker.as_ref().unwrap().thread().unpark();
        while client.completed.pop().is_err() {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        drop(client.retired.pop().ok().unwrap());
        assert_eq!(changes.load(Ordering::Acquire), RETIREMENTS + 1);
        drop(client);
        owner.finish().unwrap();
    }
}
