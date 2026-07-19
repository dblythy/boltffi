#include "boltffi_counter_host_object.h"

#include <dlfcn.h>

#include <cstdio>
#include <mutex>
#include <stdexcept>
#include <unordered_map>

using facebook::jsi::Function;
using facebook::jsi::PropNameID;
using facebook::jsi::Runtime;
using facebook::jsi::Value;

namespace boltffi {

namespace {

template <typename Fn>
Fn resolve(void* dylib, const char* name) {
  void* sym = dlsym(dylib, name);
  if (sym == nullptr) {
    throw std::runtime_error(std::string("boltffi rn_poc symbol not found: ") + name);
  }
  return reinterpret_cast<Fn>(sym);
}

// The Multiplier vtable dispatch table is process-global and constant (see the header doc: BoltFFI
// registers the trait's free/clone/method function pointers once for every object implementing
// it, then threads a caller-chosen per-instance handle id through those same functions) --
// exactly mirroring `native.bun.test.ts`'s single `boltffi_register_callback_rn_poc_multiplier`
// call outside its per-object `set_multiplier` calls.
std::mutex& multiplierRegistryMutex() {
  static std::mutex m;
  return m;
}

std::unordered_map<std::uint64_t, std::function<std::int32_t()>>& multiplierRegistry() {
  static std::unordered_map<std::uint64_t, std::function<std::int32_t()>> registry;
  return registry;
}

void multiplierFree(std::uint64_t handle) {
  std::lock_guard<std::mutex> lock(multiplierRegistryMutex());
  multiplierRegistry().erase(handle);
}

std::uint64_t multiplierClone(std::uint64_t handle) {
  // This fixture's usage (one owning `Box<dyn Multiplier>` per `Counter`) never triggers a real
  // clone; returning the same id is the safe no-op default for a trait this adapter treats as
  // move-only in practice. A future trait relying on real ref-counted clone semantics would need
  // its own registry entry per clone -- flagged, not silently assumed correct.
  return handle;
}

std::int32_t multiplierFactor(std::uint64_t handle) {
  std::function<std::int32_t()> fn;
  {
    std::lock_guard<std::mutex> lock(multiplierRegistryMutex());
    auto it = multiplierRegistry().find(handle);
    if (it == multiplierRegistry().end()) return 1;
    fn = it->second;
  }
  // Re-entrancy note (adversarial-review concern: "vtable callback re-entrancy"): this call is
  // reached synchronously from `Counter::scaled()` (a plain sync FFI call originating on the JS
  // thread), so invoking `fn()` here directly -- with no `CallInvoker` hop -- is safe: JSI
  // forbids touching a `jsi::Function` off the JS thread anyway, and we are never off it for
  // this trait's shape. `fn` itself must not re-enter Rust in a way that reallocates/removes this
  // same registry entry from under `multiplierFactor`'s caller (it doesn't -- `fn` only reads a
  // captured multiplier value in this fixture).
  return fn();
}

const MultiplierVTable& staticMultiplierVTable() {
  static const MultiplierVTable vtable{&multiplierFree, &multiplierClone, &multiplierFactor};
  return vtable;
}

}  // namespace

struct BoltFFICounterHostObject::MultiplierRegistration {
  std::uint64_t id;
};

BoltFFICounterHostObject::BoltFFICounterHostObject(
    void* dylibHandle, std::shared_ptr<facebook::react::CallInvoker> callInvoker,
    std::int32_t start)
    : dylib_(dylibHandle), callInvoker_(std::move(callInvoker)) {
  auto counterNew = resolve<std::uint64_t (*)(std::int32_t)>(
      dylib_, "boltffi_init_class_rn_poc_counter_new");
  release_ =
      resolve<void (*)(std::uint64_t)>(dylib_, "boltffi_release_class_rn_poc_counter");
  add_ = resolve<std::int32_t (*)(std::uint64_t, std::int32_t)>(
      dylib_, "boltffi_method_class_rn_poc_counter_add");
  delayedAdd_ = resolve<std::uint64_t (*)(std::uint64_t, std::int32_t)>(
      dylib_, "boltffi_method_class_rn_poc_counter_delayed_add");
  delayedAddPoll_ = resolve<void (*)(std::uint64_t, std::uint64_t, RawContinuationCallback)>(
      dylib_, "boltffi_async_method_class_rn_poc_counter_delayed_add_poll");
  delayedAddComplete_ = resolve<std::int32_t (*)(std::uint64_t, std::uint8_t*)>(
      dylib_, "boltffi_async_method_class_rn_poc_counter_delayed_add_complete");
  delayedAddFree_ = resolve<void (*)(std::uint64_t)>(
      dylib_, "boltffi_async_method_class_rn_poc_counter_delayed_add_free");
  scaled_ = resolve<std::int32_t (*)(std::uint64_t)>(
      dylib_, "boltffi_method_class_rn_poc_counter_scaled");
  setMultiplierFfi_ = resolve<std::int32_t (*)(std::uint64_t, std::uint64_t, const void*)>(
      dylib_, "boltffi_method_class_rn_poc_counter_set_multiplier");
  registerMultiplierCallback_ = resolve<void (*)(const void*)>(
      dylib_, "boltffi_register_callback_rn_poc_multiplier");

  registerMultiplierCallback_(&staticMultiplierVTable());

  handle_ = counterNew(start);
  trampoline_ = NativeContinuationTrampoline::create([this](std::function<void()> fn) {
    callInvoker_->invokeAsync(std::move(fn));
  });
}

BoltFFICounterHostObject::~BoltFFICounterHostObject() {
  if (multiplierRegistration_) {
    std::lock_guard<std::mutex> lock(multiplierRegistryMutex());
    multiplierRegistry().erase(multiplierRegistration_->id);
  }
  release_(handle_);
}

std::int32_t BoltFFICounterHostObject::addSync(std::int32_t amount) { return add_(handle_, amount); }

void BoltFFICounterHostObject::delayedAddAsync(
    std::int32_t amount, std::function<void(std::int32_t, std::int32_t)> onDone) {
  std::uint64_t future = delayedAdd_(handle_, amount);

  trampoline_->registerContinuation(
      future,
      [this](NativeHandle h, std::uint64_t callback_data, RawContinuationCallback cb) {
        delayedAddPoll_(h, callback_data, cb);
      },
      [this, onDone = std::move(onDone)](NativeHandle awaitedFuture) {
        std::uint8_t status[4] = {0, 0, 0, 0};
        std::int32_t result = delayedAddComplete_(awaitedFuture, status);
        delayedAddFree_(awaitedFuture);
        std::int32_t statusCode = status[0] | (status[1] << 8) | (status[2] << 16) | (status[3] << 24);
        onDone(result, statusCode);
      });
}

void BoltFFICounterHostObject::setMultiplier(std::function<std::int32_t()> factor) {
  auto id = reinterpret_cast<std::uint64_t>(this);
  {
    std::lock_guard<std::mutex> lock(multiplierRegistryMutex());
    multiplierRegistry()[id] = std::move(factor);
  }
  multiplierRegistration_ = std::make_unique<MultiplierRegistration>(MultiplierRegistration{id});
  setMultiplierFfi_(handle_, id, &staticMultiplierVTable());
}

std::int32_t BoltFFICounterHostObject::scaledSync() { return scaled_(handle_); }

// ---- jsi::HostObject wrapping (compiled + linked against real jsi.h; not exercised without a
// live jsi::Runtime -- see runtime/cpp/README.md's stage-4 test plan) ----

Value BoltFFICounterHostObject::get(Runtime& rt, const PropNameID& name) {
  auto propName = name.utf8(rt);

  if (propName == "add") {
    return Function::createFromHostFunction(
        rt, name, 1,
        [this](Runtime& rt2, const Value&, const Value* args, size_t) -> Value {
          return Value(addSync(static_cast<std::int32_t>(args[0].asNumber())));
        });
  }

  if (propName == "delayedAdd") {
    return Function::createFromHostFunction(
        rt, name, 1,
        [this](Runtime& rt2, const Value&, const Value* args, size_t) -> Value {
          auto amount = static_cast<std::int32_t>(args[0].asNumber());
          auto promiseCtor = rt2.global().getPropertyAsFunction(rt2, "Promise");
          auto executor = Function::createFromHostFunction(
              rt2, PropNameID::forAscii(rt2, "executor"), 2,
              [this, amount](Runtime& rt3, const Value&, const Value* execArgs, size_t) -> Value {
                auto resolve = std::make_shared<Function>(execArgs[0].asObject(rt3).asFunction(rt3));
                auto reject = std::make_shared<Function>(execArgs[1].asObject(rt3).asFunction(rt3));
                delayedAddAsync(amount, [resolve, reject](std::int32_t result, std::int32_t status) {
                  // Runs INSIDE the CallInvoker hop -- i.e. already back on the JS thread, safe
                  // to touch `Runtime`/`jsi::Function` here (unlike the raw trampoline callback).
                  //
                  // NOTE: this lambda needs a `Runtime&` to call `resolve`/`reject`, which the
                  // `CallInvoker::invokeAsync(std::function<void()>&&)` overload used by
                  // `delayedAddAsync`'s hop does not provide. A real adapter wires the
                  // `CallFunc` (`std::function<void(jsi::Runtime&)>`) overload through instead --
                  // this stage's `NativeContinuationTrampoline::ThreadHop`
                  // (`std::function<void(std::function<void()>)>`) is intentionally
                  // Runtime-agnostic (it's the JSI-INDEPENDENT layer, see native_trampoline.h),
                  // so the one-line adaptation from `CallFunc` to a Runtime-carrying completion
                  // belongs here, in the HostObject glue -- left as a documented TODO for the
                  // stage-4 pass that wires a live Runtime through this path end-to-end.
                });
                return Value::undefined();
              });
          return promiseCtor.callAsConstructor(rt2, executor);
        });
  }

  if (propName == "setMultiplier") {
    return Function::createFromHostFunction(
        rt, name, 1,
        [this](Runtime& rt2, const Value&, const Value* args, size_t) -> Value {
          auto jsFactor = std::make_shared<Function>(args[0].asObject(rt2).asFunction(rt2));
          // Safe to call `jsFactor` directly with no hop (see the header doc on
          // `setMultiplier`/`multiplierFactor`): Rust invokes this synchronously from
          // `scaled()`, itself only ever called from the JS thread. Capturing `rt2` by
          // reference is intentional, not a dangling-reference bug: every `jsi::HostObject`
          // method receives the SAME `jsi::Runtime&` for the object's whole lifetime (one
          // `Runtime` per engine instance, which always outlives the `HostObject`s it owns) --
          // `rt2` here is stable to reuse from a later, separate `scaled()` call.
          setMultiplier([jsFactor, &rt2]() -> std::int32_t {
            auto result = jsFactor->call(rt2);
            return static_cast<std::int32_t>(result.asNumber());
          });
          return Value::undefined();
        });
  }

  if (propName == "scaled") {
    return Function::createFromHostFunction(
        rt, name, 0,
        [this](Runtime& rt2, const Value&, const Value*, size_t) -> Value {
          return Value(scaledSync());
        });
  }

  return Value::undefined();
}

std::vector<PropNameID> BoltFFICounterHostObject::getPropertyNames(Runtime& rt) {
  std::vector<PropNameID> names;
  names.push_back(PropNameID::forAscii(rt, "add"));
  names.push_back(PropNameID::forAscii(rt, "delayedAdd"));
  names.push_back(PropNameID::forAscii(rt, "setMultiplier"));
  names.push_back(PropNameID::forAscii(rt, "scaled"));
  return names;
}

}  // namespace boltffi
