#include "boltffi/native_trampoline.h"

#include <utility>

namespace boltffi {

namespace {

// Meyer's singletons (function-local statics) to sidestep static-init-order issues: exactly one
// trampoline may be "active" (able to receive raw callbacks) at a time in a process, mirroring
// the ABI's own constraint -- `RawContinuationCallback` is a plain function pointer with no
// per-instance userdata slot, so `rawTrampoline` must resolve "which manager" through a single
// process-wide slot rather than a closure capture.
std::mutex& slotMutex() {
  static std::mutex mutex;
  return mutex;
}

std::weak_ptr<NativeContinuationTrampoline>& activeSlot() {
  static std::weak_ptr<NativeContinuationTrampoline> slot;
  return slot;
}

}  // namespace

NativeContinuationTrampoline::NativeContinuationTrampoline(ThreadHop hop) : hop_(std::move(hop)) {}

std::shared_ptr<NativeContinuationTrampoline> NativeContinuationTrampoline::create(ThreadHop hop) {
  // NOTE: not `make_shared` -- the constructor is private, and `create` (a member function) has
  // access to it directly.
  std::shared_ptr<NativeContinuationTrampoline> self(
      new NativeContinuationTrampoline(std::move(hop)));
  {
    std::lock_guard<std::mutex> lock(slotMutex());
    activeSlot() = self;
  }
  return self;
}

NativeContinuationTrampoline::~NativeContinuationTrampoline() {
  // Teardown safety (adversarial-review concern: "module unload with pending continuations"):
  // clear the slot only if it still points at `this`. Because `activeSlot()` holds a `weak_ptr`,
  // and this destructor only runs once the LAST owning `shared_ptr` is released, no `onSignal`
  // call can be concurrently "in flight" against `this` at this point -- any such call would
  // itself be holding a `shared_ptr` (via `rawTrampoline`'s local `self` or the hop lambda's
  // captured `self`), which would keep the refcount above zero and this destructor from running
  // at all yet. The lock here only guards against a *new* `rawTrampoline` call racing to read
  // `activeSlot()` while we clear it.
  std::lock_guard<std::mutex> lock(slotMutex());
  if (activeSlot().lock().get() == this) {
    activeSlot().reset();
  }
}

std::uint64_t NativeContinuationTrampoline::registerContinuation(
    NativeHandle handle,
    std::function<void(NativeHandle, std::uint64_t, RawContinuationCallback)> repoll,
    std::function<void(NativeHandle)> resolve) {
  std::uint64_t callback_data;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    callback_data = next_callback_data_++;
    pending_.emplace(callback_data,
                      PendingContinuation{handle, repoll, std::move(resolve)});
  }
  // Mirrors `NativeAsyncFutureManager.pollAsyncNative` (native.ts): the pending entry exists in
  // the table BEFORE `repoll` runs, so a same-call synchronous signal (the "Waked" race, where
  // the real Rust future resolves inside this very registration call) is found by `onSignal`
  // instead of being dropped as an unknown `callback_data`.
  repoll(handle, callback_data, trampolineFunction());
  return callback_data;
}

RawContinuationCallback NativeContinuationTrampoline::trampolineFunction() const {
  return &NativeContinuationTrampoline::rawTrampoline;
}

std::size_t NativeContinuationTrampoline::pendingCount() const {
  std::lock_guard<std::mutex> lock(mutex_);
  return pending_.size();
}

void NativeContinuationTrampoline::rawTrampoline(std::uint64_t callback_data, std::int8_t signal) {
  // The absolute minimum on whatever thread Rust's waker fires this on: resolve which manager
  // (if any) is still alive, then leave immediately -- no JSI, no allocation beyond the
  // shared_ptr refcount bump.
  std::shared_ptr<NativeContinuationTrampoline> self;
  {
    std::lock_guard<std::mutex> lock(slotMutex());
    self = activeSlot().lock();
  }
  if (!self) {
    // The owning module was already torn down (or never installed) -- drop the signal silently.
    // The raw callback thread must never be where an exception/crash originates (mirrors
    // native.ts's `onSignal`: "dropped, never thrown").
    return;
  }
  self->onSignal(callback_data, static_cast<NativeContinuationSignal>(signal));
}

void NativeContinuationTrampoline::onSignal(std::uint64_t callback_data,
                                             NativeContinuationSignal signal) {
  if (signal == NativeContinuationSignal::MaybeReady) {
    std::function<void(NativeHandle, std::uint64_t, RawContinuationCallback)> repoll;
    NativeHandle handle{};
    {
      std::lock_guard<std::mutex> lock(mutex_);
      auto it = pending_.find(callback_data);
      if (it == pending_.end()) return;
      repoll = it->second.repoll;
      handle = it->second.handle;
    }
    // Re-issue the FFI poll() call directly, on whichever thread this signal fired on -- no
    // mutex held (repoll may synchronously re-enter `rawTrampoline`/`onSignal` if the future
    // resolves immediately; holding a lock here would deadlock that reentrant call). Matches
    // native.ts's own "no microtask deferral" comment: registration only touches the lock-free
    // ContinuationScheduler on the Rust side, so calling it straight back is safe.
    repoll(handle, callback_data, trampolineFunction());
    return;
  }

  std::function<void(NativeHandle)> resolve;
  NativeHandle handle{};
  {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = pending_.find(callback_data);
    if (it == pending_.end()) return;
    resolve = std::move(it->second.resolve);
    handle = it->second.handle;
    pending_.erase(it);
  }
  // `self` keeps this object alive for as long as the hop lambda hasn't run yet, even if every
  // external owner (and the process-wide slot) has already let go in the meantime -- the
  // teardown-safety half of the same concern the destructor's doc covers.
  auto self = shared_from_this();
  hop_([self, resolve, handle]() { resolve(handle); });
}

}  // namespace boltffi
