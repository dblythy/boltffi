// The C++ counterpart to `@boltffi/runtime`'s `NativeAsyncFutureManager`
// (runtime/typescript/src/native.ts) -- the react-native track's stage-3 JSI adapter
// (docs/tracks/react-native.md, parse-core-sdks repo). Deliberately free of any `jsi::` type:
// this is the "JSI-independent part" the design calls out as pure C++ unit-testable without a
// live JS engine (buffer decode, trampoline dispatch, handle-table safety). The JSI-coupled glue
// (constructing the real `facebook::react::CallInvoker::invokeAsync` hop, decoding results into
// `jsi::Value`) lives one layer up, in jsi/boltffi_counter_host_object.h.
//
// Mirrors native.ts's own doc: the native async protocol is a REGISTRATION
// (`boltffi_core::runtime::continuation::ContinuationScheduler`), not a synchronous query --
// `RustFuture::poll` may invoke the registered continuation callback synchronously, inside the
// very call that registered it (the "Waked" race), or later from an arbitrary OS thread the real
// future's `Waker` fires on (`future.rs:333-341`). This header's contract:
//   1. The pending-registration table entry is inserted BEFORE the underlying FFI `poll()` call
//      runs, exactly like `NativeAsyncFutureManager.pollAsyncNative` -- so the synchronous race
//      resolves correctly (the entry already exists when the same-call signal arrives).
//   2. `onSignal` (reached through the one fixed `RawCallback` function pointer every pending
//      call shares) does the absolute minimum on the calling thread: look up state under a mutex,
//      then hand off through the injected `ThreadHop` -- it never assumes which thread it runs
//      on, and the JSI layer's `ThreadHop` implementation is the ONLY place that touches
//      `jsi::Runtime`.
//   3. Teardown safety: because `RawCallback` is a plain `extern "C"` function pointer (no
//      per-call userdata slot in the ABI -- `RustFutureContinuationCallback` is
//      `extern "C" fn(u64, i8)`, header.h), only one trampoline can be "active" at a time in a
//      process; `NativeContinuationTrampoline` holds its state behind a `shared_ptr` and installs
//      a `weak_ptr` in the global slot, so a signal racing the owning object's destruction either
//      observes the slot already cleared (dropped, no crash) or locks a still-valid `shared_ptr`
//      that keeps the state alive for the duration of that one call even if the destructor has
//      already started running on another thread.
#pragma once

#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <unordered_map>

namespace boltffi {

/// Matches `boltffi_core::runtime::future::RustFuturePoll` (`extern "C" fn(u64, i8)`) --
/// mirrors `native.ts`'s `NativeContinuationSignal`.
enum class NativeContinuationSignal : int8_t {
  Ready = 0,
  MaybeReady = 1,
};

/// Opaque native future/class handle -- always a real process address or an opaque id, never
/// surfaced to JS as a `number` (see the design doc's handle-safety note). Mirrors `native.ts`'s
/// `NativeHandle` (there a `bigint`; here the raw 64-bit value the JSI layer wraps as a
/// `jsi::BigInt` only when it must cross into JS).
using NativeHandle = std::uint64_t;

/// The one fixed, generic `extern "C"`-compatible continuation callback shape every async
/// method's `poll` registration reuses for the lifetime of a trampoline instance.
using RawContinuationCallback = void (*)(std::uint64_t /*callback_data*/, std::int8_t /*signal*/);

/// Schedules `fn` to run on the JS-owning thread. The real JSI adapter implements this as a thin
/// wrapper over `facebook::react::CallInvoker::invokeAsync`; unit tests inject a synchronous or a
/// real-background-thread fake to exercise both the same-call and off-thread races without any
/// JSI dependency at all.
using ThreadHop = std::function<void(std::function<void()>)>;

/// One pending registration: the FFI poll-call to re-issue on `MaybeReady`, and the resolution
/// callback to invoke once `Ready`. Mirrors `native.ts`'s `NativePendingFuture`.
struct PendingContinuation {
  NativeHandle handle;
  std::function<void(NativeHandle, std::uint64_t /*callback_data*/, RawContinuationCallback)> repoll;
  std::function<void(NativeHandle)> resolve;
};

/// The native-protocol continuation trampoline (design doc: "the C++ continuation trampoline is
/// one fixed, generic `extern "C"` function, reused for every async method"). Owns exactly one
/// pending-registration table, keyed by a manager-assigned `callback_data` id (never the raw
/// `handle` -- multiple in-flight calls against unrelated handles must never collide).
class NativeContinuationTrampoline
    : public std::enable_shared_from_this<NativeContinuationTrampoline> {
 public:
  /// Constructs a trampoline that hops through `hop` before ever touching JS-owned state.
  /// Installs itself as the process's active trampoline (see the class doc's teardown note) --
  /// only one instance should be alive at a time in a real adapter (one per JS runtime), though
  /// tests may construct/destroy several in sequence.
  static std::shared_ptr<NativeContinuationTrampoline> create(ThreadHop hop);

  ~NativeContinuationTrampoline();

  NativeContinuationTrampoline(const NativeContinuationTrampoline&) = delete;
  NativeContinuationTrampoline& operator=(const NativeContinuationTrampoline&) = delete;

  /// Registers `handle`'s continuation and immediately invokes `repoll` (synchronously, on the
  /// calling thread) with the fixed trampoline function pointer -- mirrors
  /// `NativeAsyncFutureManager.pollAsyncNative`: the pending entry is already in the table before
  /// `repoll` runs, so a same-call synchronous `Ready` signal resolves correctly instead of being
  /// dropped as "unknown callback_data".
  std::uint64_t registerContinuation(
      NativeHandle handle,
      std::function<void(NativeHandle, std::uint64_t, RawContinuationCallback)> repoll,
      std::function<void(NativeHandle)> resolve);

  /// The fixed function pointer to pass as the native ABI's continuation callback parameter.
  RawContinuationCallback trampolineFunction() const;

  /// Test/diagnostic hook: number of continuations still awaiting a terminal signal.
  std::size_t pendingCount() const;

 private:
  explicit NativeContinuationTrampoline(ThreadHop hop);

  void onSignal(std::uint64_t callback_data, NativeContinuationSignal signal);
  static void rawTrampoline(std::uint64_t callback_data, std::int8_t signal);

  ThreadHop hop_;
  mutable std::mutex mutex_;
  std::unordered_map<std::uint64_t, PendingContinuation> pending_;
  std::uint64_t next_callback_data_ = 1;
};

}  // namespace boltffi
