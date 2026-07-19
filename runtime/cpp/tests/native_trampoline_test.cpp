// Pure C++ unit tests for boltffi/native_trampoline.h -- the "JSI-independent part" of the
// react-native stage-3 adapter (docs/tracks/react-native.md). No jsi:: type, no Rust toolchain,
// no live JS engine: exercises the trampoline's registration/signal/teardown contract directly,
// covering exactly the adversarial-review attack surface the design doc calls out (the same-call
// "Waked" race, off-thread delivery, and module teardown with pending continuations).
#include "boltffi/native_trampoline.h"

#include <atomic>
#include <chrono>
#include <thread>

#include "test_harness.h"

using boltffi::NativeContinuationSignal;
using boltffi::NativeContinuationTrampoline;
using boltffi::NativeHandle;
using boltffi::RawContinuationCallback;

namespace {

// A synchronous hop: runs `fn` immediately, on the calling thread. Exercises the same-call race
// path -- resolution happens with no thread hop latency at all.
boltffi::ThreadHop syncHop() {
  return [](std::function<void()> fn) { fn(); };
}

}  // namespace

BOLTFFI_TEST(registration_before_repoll_resolves_the_synchronous_waked_race) {
  // Mirrors the design doc's "Waked" race: the real Rust future resolves INSIDE the very poll()
  // call that registered it, on the calling thread. `repoll` here simulates that by invoking the
  // trampoline function pointer with Ready synchronously, before `registerContinuation` returns.
  auto hop_calls = std::make_shared<int>(0);
  auto trampoline = NativeContinuationTrampoline::create(
      [hop_calls](std::function<void()> fn) {
        ++*hop_calls;
        fn();
      });

  bool resolved = false;
  NativeHandle resolved_handle = 0;

  trampoline->registerContinuation(
      /*handle=*/42,
      [](NativeHandle h, std::uint64_t callback_data, RawContinuationCallback cb) {
        // Simulates Rust: the poll() call itself invokes the continuation callback
        // synchronously, before returning control to the caller.
        cb(callback_data, static_cast<std::int8_t>(NativeContinuationSignal::Ready));
      },
      [&](NativeHandle h) {
        resolved = true;
        resolved_handle = h;
      });

  BOLTFFI_CHECK(resolved);
  BOLTFFI_CHECK(resolved_handle == 42);
  BOLTFFI_CHECK(*hop_calls == 1);
  BOLTFFI_CHECK(trampoline->pendingCount() == 0);
}

BOLTFFI_TEST(maybe_ready_repolls_directly_then_ready_resolves_through_the_hop) {
  int repoll_count = 0;
  bool resolved = false;

  auto trampoline = NativeContinuationTrampoline::create(syncHop());

  trampoline->registerContinuation(
      /*handle=*/7,
      [&](NativeHandle h, std::uint64_t callback_data, RawContinuationCallback cb) {
        ++repoll_count;
        if (repoll_count < 3) {
          cb(callback_data, static_cast<std::int8_t>(NativeContinuationSignal::MaybeReady));
        } else {
          cb(callback_data, static_cast<std::int8_t>(NativeContinuationSignal::Ready));
        }
      },
      [&](NativeHandle) { resolved = true; });

  BOLTFFI_CHECK(repoll_count == 3);
  BOLTFFI_CHECK(resolved);
  BOLTFFI_CHECK(trampoline->pendingCount() == 0);
}

BOLTFFI_TEST(off_thread_signal_delivery_resolves_safely) {
  // Proves the trampoline is safe to invoke from a thread the manager never touched -- the real
  // Rust waker fires the raw callback from an arbitrary OS thread (future.rs's own
  // `delayed_wake_future_completes_through_exported_handle` test; this repo's PoC evidence).
  std::atomic<bool> resolved{false};
  std::atomic<bool> hop_ran_on_hop_thread{false};

  auto trampoline = NativeContinuationTrampoline::create(
      [](std::function<void()> fn) {
        // A "hop" that actually spawns a thread -- stands in for
        // CallInvoker::invokeAsync bouncing onto the JS thread.
        std::thread([fn] { fn(); }).detach();
      });

  RawContinuationCallback saved_cb = nullptr;
  std::uint64_t saved_callback_data = 0;

  trampoline->registerContinuation(
      /*handle=*/99,
      [&](NativeHandle, std::uint64_t callback_data, RawContinuationCallback cb) {
        // Real async case: poll() returns without signalling -- the signal arrives later,
        // off-thread.
        saved_cb = cb;
        saved_callback_data = callback_data;
      },
      [&](NativeHandle) {
        resolved = true;
        hop_ran_on_hop_thread = true;
      });

  BOLTFFI_CHECK(!resolved.load());
  BOLTFFI_CHECK(trampoline->pendingCount() == 1);

  std::thread waker([&] {
    std::this_thread::sleep_for(std::chrono::milliseconds(20));
    saved_cb(saved_callback_data, static_cast<std::int8_t>(NativeContinuationSignal::Ready));
  });
  waker.join();

  for (int i = 0; i < 200 && !resolved.load(); ++i) {
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }

  BOLTFFI_CHECK(resolved.load());
  BOLTFFI_CHECK(trampoline->pendingCount() == 0);
}

BOLTFFI_TEST(unknown_callback_data_is_dropped_not_thrown) {
  auto trampoline = NativeContinuationTrampoline::create(syncHop());
  RawContinuationCallback cb = trampoline->trampolineFunction();

  // No registration ever happened for id 12345 -- must be a silent no-op, never a crash or
  // exception escaping across the raw callback boundary.
  cb(12345, static_cast<std::int8_t>(NativeContinuationSignal::Ready));
  BOLTFFI_CHECK(trampoline->pendingCount() == 0);
}

BOLTFFI_TEST(signal_after_teardown_is_dropped_not_a_use_after_free) {
  // Adversarial-review concern: "the CallInvoker hop under teardown -- module unload with
  // pending continuations." Save the raw function pointer and a callback_data for a still-
  // pending registration, destroy the owning trampoline (simulating the JSI module/HostObject
  // being torn down), then invoke the stale pointer -- must not crash or resolve anything.
  RawContinuationCallback stale_cb;
  std::uint64_t stale_callback_data = 0;
  bool resolved_after_teardown = false;

  {
    auto trampoline = NativeContinuationTrampoline::create(syncHop());
    stale_cb = trampoline->trampolineFunction();
    trampoline->registerContinuation(
        /*handle=*/1,
        [&](NativeHandle, std::uint64_t callback_data, RawContinuationCallback) {
          stale_callback_data = callback_data;
        },
        [&](NativeHandle) { resolved_after_teardown = true; });
    // `trampoline` (the only shared_ptr) goes out of scope here -- the manager is destroyed
    // with one continuation still pending.
  }

  // The static trampoline function is still a valid function pointer (it's not per-instance),
  // but the process-wide active slot has been cleared by the destructor -- this must be a
  // no-op, not a use-after-free.
  stale_cb(stale_callback_data, static_cast<std::int8_t>(NativeContinuationSignal::Ready));
  BOLTFFI_CHECK(!resolved_after_teardown);
}

BOLTFFI_TEST(a_new_trampoline_after_teardown_works_normally) {
  // The process-wide active-slot design (one trampoline "active" at a time) must not leave the
  // slot poisoned after a previous instance tears down -- a fresh instance should work exactly
  // as if it were the first ever constructed.
  {
    auto first = NativeContinuationTrampoline::create(syncHop());
    (void)first;
  }

  bool resolved = false;
  auto second = NativeContinuationTrampoline::create(syncHop());
  second->registerContinuation(
      /*handle=*/5,
      [](NativeHandle h, std::uint64_t callback_data, RawContinuationCallback cb) {
        cb(callback_data, static_cast<std::int8_t>(NativeContinuationSignal::Ready));
      },
      [&](NativeHandle) { resolved = true; });

  BOLTFFI_CHECK(resolved);
}

int main() { return boltffi_test::runAll(); }
