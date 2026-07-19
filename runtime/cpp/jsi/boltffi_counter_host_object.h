// The stage-3 JSI adapter's first real slice (docs/tracks/react-native.md): a `jsi::HostObject`
// driving the rn_poc fixture crate's real compiled C ABI (`runtime/typescript/test/fixtures/
// rn_poc`) -- the same `Counter`/`Multiplier` shapes `native.bun.test.ts` already proves against
// Bun's FFI, here proven against a REAL `facebook::jsi::HostObject` instead of a stand-in host.
//
// Split in two halves on purpose:
//   - The "core" methods (`addSync`, `delayedAddAsync`, `setMultiplier`, `scaledSync`) touch only
//     the C ABI + `boltffi::NativeContinuationTrampoline` + `facebook::react::CallInvoker` --
//     they take/return plain C++ types, so a test can drive them directly with a fake
//     `CallInvoker` and no live `jsi::Runtime` at all (see jsi/linkcheck_main.cpp).
//   - `get()`/`getPropertyNames()` are the thin, mechanical `jsi::Value`<->native wrapping layer
//     every HybridObject-style method needs -- real code, compiled and linked against the actual
//     `facebook::jsi` headers/library, but not independently unit-testable without a live engine
//     (Hermes/JSC), which this stage's CMake target set deliberately does not vendor (see
//     runtime/cpp/README.md's stage-4 test plan).
#pragma once

#include <ReactCommon/CallInvoker.h>
#include <jsi/jsi.h>

#include <cstdint>
#include <functional>
#include <memory>

#include "boltffi/native_trampoline.h"

namespace boltffi {

/// `BoltFFICallbackHandle`-shaped vtable slot order (free, clone, then the trait's own declared
/// methods) -- matches `native.bun.test.ts`'s own comment on the generated C# `Multiplier.cs`
/// vtable layout: BoltFFI attaches the standard free/clone lifecycle pair before a callback
/// trait's declared methods.
struct MultiplierVTable {
  void (*free)(std::uint64_t handle);
  std::uint64_t (*clone)(std::uint64_t handle);
  std::int32_t (*factor)(std::uint64_t handle);
};

class BoltFFICounterHostObject : public facebook::jsi::HostObject {
 public:
  BoltFFICounterHostObject(void* dylibHandle,
                            std::shared_ptr<facebook::react::CallInvoker> callInvoker,
                            std::int32_t start);
  ~BoltFFICounterHostObject() override;

  BoltFFICounterHostObject(const BoltFFICounterHostObject&) = delete;
  BoltFFICounterHostObject& operator=(const BoltFFICounterHostObject&) = delete;

  // ---- jsi::HostObject overrides (the mechanical wrapping layer) ----
  facebook::jsi::Value get(facebook::jsi::Runtime& rt,
                            const facebook::jsi::PropNameID& name) override;
  std::vector<facebook::jsi::PropNameID> getPropertyNames(facebook::jsi::Runtime& rt) override;

  // ---- The JSI-independent core (directly testable, see jsi/linkcheck_main.cpp) ----

  /// `Counter::add` -- plain scalar-in/scalar-out sync call.
  std::int32_t addSync(std::int32_t amount);

  /// `Counter::delayed_add` -- the real native-async continuation protocol (register -> off-
  /// thread callback -> repoll -> complete), through `NativeContinuationTrampoline`. `onDone` is
  /// invoked (through `callInvoker_->invokeAsync`, i.e. always hopped, never on the calling
  /// thread directly) with the result, or `std::nullopt` plus a status code on failure.
  void delayedAddAsync(std::int32_t amount,
                        std::function<void(std::int32_t /*result*/, std::int32_t /*status*/)> onDone);

  /// `Counter::set_multiplier` -- registers a host (JS)-implemented callback. `factor` stands in
  /// for what the real adapter would wrap around a `jsi::Function`; it is called SYNCHRONOUSLY,
  /// on whatever thread `Counter::scaled()` is invoked from (a plain sync FFI call), which in the
  /// real adapter is always the JS thread (JSI forbids calling a `jsi::Function` from any other
  /// thread) -- so, unlike the async continuation, this path deliberately does NOT hop through
  /// `callInvoker_`. If a future host-callback trait could ever be invoked from a non-JS thread
  /// (an async callback method, none of which this fixture uses), it would need the same hop
  /// discipline as `delayedAddAsync` -- flagged, not silently assumed away.
  void setMultiplier(std::function<std::int32_t()> factor);

  /// `Counter::scaled` -- plain sync call that (if a multiplier is registered) re-enters through
  /// the vtable's `factor` slot synchronously, on this same call stack.
  std::int32_t scaledSync();

 private:
  void* dylib_;
  std::uint64_t handle_;
  std::shared_ptr<facebook::react::CallInvoker> callInvoker_;
  std::shared_ptr<NativeContinuationTrampoline> trampoline_;

  // Resolved rn_poc symbols (see rn_poc/src/lib.rs / native.bun.test.ts's `bindExports`).
  void (*release_)(std::uint64_t);
  std::int32_t (*add_)(std::uint64_t, std::int32_t);
  std::uint64_t (*delayedAdd_)(std::uint64_t, std::int32_t);
  void (*delayedAddPoll_)(std::uint64_t, std::uint64_t, RawContinuationCallback);
  std::int32_t (*delayedAddComplete_)(std::uint64_t, std::uint8_t*);
  void (*delayedAddFree_)(std::uint64_t);
  std::int32_t (*scaled_)(std::uint64_t);
  std::int32_t (*setMultiplierFfi_)(std::uint64_t, std::uint64_t, const void*);
  void (*registerMultiplierCallback_)(const void*);

  // Owns the currently-registered JS-side multiplier callback and its vtable, if any -- kept
  // alive for as long as Rust might call back into it (freed only when the vtable's own `free`
  // slot fires, mirroring the real ownership transfer `Box<dyn Multiplier>` performs).
  struct MultiplierRegistration;
  std::unique_ptr<MultiplierRegistration> multiplierRegistration_;
};

}  // namespace boltffi
