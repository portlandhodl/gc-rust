//! Synchronization between LWP threads — wait-queues, mutexes, bounded
//! message queues. Bedrock for the "background agent" pattern (park the
//! agent on a queue, get woken by the UI/network thread).
//!
//! No spin-locks: everything parks on the LWP scheduler.

use crate::lwp;

/// A wait channel: threads `wait()` on it; anyone `wake_all()`ing lets them
/// re-check. Use this for "work arrived" style signals (queues below use
/// exactly that).
pub struct WaitQueue {
    token: u32,
}

static NEXT_TOKEN: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0x1000_0000);

impl WaitQueue {
    pub fn new() -> WaitQueue {
        WaitQueue { token: NEXT_TOKEN.fetch_add(1, core::sync::atomic::Ordering::Relaxed) }
    }

    /// Park the calling thread until the queue is woken. Returns
    /// immediately if no scheduler is running (single-threaded app).
    pub fn wait(&self) {
        if !lwp::scheduler_running() {
            return;
        }
        lwp::block_current(self.token);
    }

    /// Wake everybody parked on this queue.
    pub fn wake_all(&self) {
        if !lwp::scheduler_running() {
            return;
        }
        lwp::wake_token(self.token);
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Sleeping mutex (blocks, doesn't spin).
pub struct Mutex {
    owner: core::sync::atomic::AtomicU32, // 0 = free; else 1 + thread idx
    gate: WaitQueue,
}

impl Mutex {
    fn my_id() -> u32 {
        lwp::current_id().saturating_add(1)
    }

    pub fn new() -> Mutex {
        Mutex { owner: core::sync::atomic::AtomicU32::new(0), gate: WaitQueue::new() }
    }

    pub fn lock(&self) {
        if !lwp::scheduler_running() {
            return;
        }
        let me = Self::my_id();
        loop {
            match self.owner.compare_exchange(
                0,
                me,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(_) => self.gate.wait(),
            }
        }
    }

    pub fn unlock(&self) {
        if !lwp::scheduler_running() {
            return;
        }
        let me = Self::my_id();
        if self.owner.swap(0, core::sync::atomic::Ordering::Release) == me {
            self.gate.wake_all();
        }
    }
}

impl Default for Mutex {
    fn default() -> Self {
        Self::new()
    }
}

/// Bounded blocking message queue for inter-thread work items. `T: Copy`.
pub struct Channel<T: Copy> {
    items: *mut T,
    capacity: usize,
    head: usize,
    tail: usize,
    count: usize,
    full_q: WaitQueue,
    empty_q: WaitQueue,
}

impl<T: Copy> Channel<T> {
    /// Allocate a channel of `capacity` items (leaked, caller-chosen).
    /// `new_inner` is the runtime constructor for [`Channel`]; store it
    /// inside your own struct and hand out `&mut` references.
    pub fn into_self(capacity: usize) -> Channel<T> {
        let items = unsafe {
            alloc::alloc::alloc(
                alloc::alloc::Layout::array::<T>(capacity).unwrap()
                    .align_to(4).unwrap().pad_to_align(),
            ) as *mut T
        };
        Channel {
            items,
            capacity,
            head: 0,
            tail: 0,
            count: 0,
            full_q: WaitQueue::new(),
            empty_q: WaitQueue::new(),
        }
    }

    /// Send (blocks while full).
    pub fn send(&mut self, item: T) {
        if !lwp::scheduler_running() {
            // main-thread only: drop silently if no scheduler (agent packets
            // only make sense while threads are up)
            return;
        }
        loop {
            {
                let _g = crate::irq::IrqLock::take();
                if self.count < self.capacity {
                    unsafe { *self.items.add(self.tail) = item; }
                    self.tail = (self.tail + 1) % self.capacity;
                    self.count += 1;
                    drop(_g);
                    self.empty_q.wake_all();
                    return;
                }
            }
            self.full_q.wait();
        }
    }

    /// Receive (blocks while empty).
    pub fn recv(&mut self) -> Option<T> {
        if !lwp::scheduler_running() {
            return None;
        }
        loop {
            {
                let _g = crate::irq::IrqLock::take();
                if self.count > 0 {
                    let v = unsafe { *self.items.add(self.head) };
                    self.head = (self.head + 1) % self.capacity;
                    self.count -= 1;
                    drop(_g);
                    self.full_q.wake_all();
                    return Some(v);
                }
            }
            self.empty_q.wait();
        }
    }

    /// Non-blocking read.
    pub fn try_recv(&mut self) -> Option<T> {
        {
            if self.count == 0 {
                None
            } else {
                let _g = crate::irq::IrqLock::take();
                let v = unsafe { *self.items.add(self.head) };
                self.head = (self.head + 1) % self.capacity;
                self.count -= 1;
                Some(v)
            }
        }
    }
}
extern crate alloc;
