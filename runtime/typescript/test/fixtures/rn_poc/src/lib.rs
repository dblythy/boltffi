//! Fixture crate for the `@boltffi/runtime` native-backend PoC.
//!
//! `Counter` exercises the three shapes stage 1's runtime native backend needs proof against:
//! a sync method (`add`), an async method whose future genuinely wakes from a background OS
//! thread (`delayed_add`, mirroring `boltffi_core::runtime::future`'s own
//! `delayed_wake_future_completes_through_exported_handle` test), and a host-implemented
//! callback trait (`Multiplier`) the host registers and Rust calls back into synchronously.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

#[boltffi::export]
pub trait Multiplier: Send + Sync {
    fn factor(&self) -> i32;
}

struct DelayedAddState {
    ready: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// A future that only becomes ready after a real background thread sleeps and wakes it —
/// the same off-thread-wake shape `RustFuture`'s continuation protocol must survive.
struct DelayedAdd {
    state: Arc<DelayedAddState>,
    amount: i32,
    spawned: bool,
}

impl DelayedAdd {
    fn new(amount: i32) -> Self {
        Self {
            state: Arc::new(DelayedAddState {
                ready: AtomicBool::new(false),
                waker: Mutex::new(None),
            }),
            amount,
            spawned: false,
        }
    }
}

impl Future for DelayedAdd {
    type Output = i32;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.state.ready.load(Ordering::Acquire) {
            return Poll::Ready(this.amount);
        }
        *this.state.waker.lock().unwrap() = Some(cx.waker().clone());
        if !this.spawned {
            this.spawned = true;
            let state = Arc::clone(&this.state);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(150));
                state.ready.store(true, Ordering::Release);
                if let Some(waker) = state.waker.lock().unwrap().take() {
                    waker.wake();
                }
            });
        }
        Poll::Pending
    }
}

pub struct Counter {
    value: AtomicI32,
    multiplier: Mutex<Option<Box<dyn Multiplier>>>,
}

#[boltffi::export]
impl Counter {
    pub fn new(start: i32) -> Self {
        Self {
            value: AtomicI32::new(start),
            multiplier: Mutex::new(None),
        }
    }

    /// Sync method: proves the native backend's plain scalar-in/scalar-out call path.
    pub fn add(&self, amount: i32) -> i32 {
        self.value.fetch_add(amount, Ordering::SeqCst) + amount
    }

    /// Async method whose future wakes from a real background thread: proves the native
    /// continuation-callback protocol (register -> off-thread callback -> repoll -> complete).
    pub async fn delayed_add(&self, amount: i32) -> i32 {
        let added = DelayedAdd::new(amount).await;
        self.value.fetch_add(added, Ordering::SeqCst) + added
    }

    /// Registers a host (JS)-implemented callback: proves the host-callback round trip.
    pub fn set_multiplier(&self, multiplier: Box<dyn Multiplier>) {
        *self.multiplier.lock().unwrap() = Some(multiplier);
    }

    /// Calls back into the host-registered callback synchronously.
    pub fn scaled(&self) -> i32 {
        let factor = self
            .multiplier
            .lock()
            .unwrap()
            .as_ref()
            .map(|m| m.factor())
            .unwrap_or(1);
        self.value.load(Ordering::SeqCst) * factor
    }
}
