//! Deferred release of foreign (host-implemented) callback handles.
//!
//! A `Foreign*` wrapper's `Drop` can run in contexts where re-entering the host runtime is
//! forbidden outright — Dart runs `NativeFinalizer`s in a GC context whose VM check aborts the
//! process on ANY native-callback invocation ("Cannot invoke native callback from a leaf call"),
//! regardless of how the symbol was declared. Dispatching the vtable's free slot from a plain OS
//! thread is always legal (listener-style trampolines are designed for arbitrary-thread calls),
//! so `Drop` enqueues here instead of calling synchronously.
//!
//! The closure re-checks its trait's dead-vtable gate at execution time, so a release that
//! races host teardown degrades to a skipped (leaked) handle-map entry, never a wild call.

use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;

static REAPER: OnceLock<Sender<Box<dyn FnOnce() + Send>>> = OnceLock::new();

/// Run `release` on the shared reaper thread (started on first use). Never blocks; if the
/// channel is closed (only possible during process teardown) the release is skipped — the OS is
/// about to reclaim everything anyway.
pub fn defer_release(release: Box<dyn FnOnce() + Send>) {
    let sender = REAPER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        std::thread::Builder::new()
            .name("boltffi-callback-reaper".into())
            .spawn(move || {
                while let Ok(release) = rx.recv() {
                    release();
                }
            })
            .ok();
        tx
    });
    let _ = sender.send(release);
}
