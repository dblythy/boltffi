// End-to-end proof, against a REAL compiled native dylib (rn_poc, built through the same
// experimental BindingExpansion macro path `dist/apple`/`dist/android` use -- see
// runtime/typescript/test/fixtures/build-rn-poc.sh), that:
//   (A) the generic, name-driven dispatcher (abi_header.h + generic_invoke.h) can construct an
//       object, call a sync method, and drive a real async method (register -> off-thread wake ->
//       repoll -> sret-aggregate complete -> free) WITHOUT any per-symbol C++ code -- only dlsym by
//       name and the small closed shape library `generic_invoke.h` documents.
//   (B) the generic callback-vtable builder (generic_callback.h) can register a REAL host-callback
//       trait implementation (`AsyncKv`, added this session specifically to mirror
//       `SessionStorage`'s real async-completion shape) and have Rust successfully call back into a
//       JS-side handler simulated by a background thread -- proving the host-callback bridge's
//       hardest case (async completion, off-thread, decoded via the real wire format) end to end.
//
// This is NOT run by the default `ctest` target (it needs a real Rust build, `bash
// runtime/typescript/test/fixtures/build-rn-poc.sh`, which this repo's DISK discipline keeps out of
// version control) -- CMakeLists.txt only registers it when `BOLTFFI_RN_POC_DYLIB` is set to the
// built artifact's path, mirroring the JSI-adapter targets' `BOLTFFI_JSI_INCLUDE_DIR` gate.
#include <dlfcn.h>

#include <atomic>
#include <chrono>
#include <cstring>
#include <thread>

#include "boltffi/abi_header.h"
#include "boltffi/generic_callback.h"
#include "boltffi/generic_invoke.h"
#include "boltffi/native_trampoline.h"
#include "test_harness.h"

using namespace boltffi;

namespace {

void* gDylib = nullptr;

void* resolve(const char* name) {
  void* sym = dlsym(gDylib, name);
  if (!sym) throw std::runtime_error(std::string("symbol not found: ") + name);
  return sym;
}

// ---- (A) generic dispatch: construct + sync call, name-driven, no per-symbol C++ ----

BOLTFFI_TEST(generic_dispatch_constructs_and_calls_sync_method) {
  void* ctor = resolve("boltffi_init_class_rn_poc_counter_new");
  CValue ctorArgs[1] = {CValue::ofU64(10)};
  std::uint64_t counter = invokeGenericScalar(ctor, ctorArgs, 1);
  BOLTFFI_CHECK(counter != 0);

  void* addFn = resolve("boltffi_method_class_rn_poc_counter_add");
  CValue addArgs[2] = {CValue::ofU64(counter), CValue::ofU64(5)};
  auto result = invokeGenericScalar(addFn, addArgs, 2);
  BOLTFFI_CHECK(static_cast<std::int32_t>(result) == 15);

  void* release = resolve("boltffi_release_class_rn_poc_counter");
  CValue releaseArgs[1] = {CValue::ofU64(counter)};
  invokeGenericVoid(release, releaseArgs, 1);
}

// ---- (A) generic dispatch: the real async-future protocol (register/wake/repoll/sret-complete) ----

BOLTFFI_TEST(generic_dispatch_drives_real_async_future_to_completion) {
  void* ctor = resolve("boltffi_init_class_rn_poc_counter_new");
  CValue ctorArgs[1] = {CValue::ofU64(20)};
  std::uint64_t counter = invokeGenericScalar(ctor, ctorArgs, 1);

  void* entry = resolve("boltffi_method_class_rn_poc_counter_delayed_add");
  void* poll = resolve("boltffi_async_method_class_rn_poc_counter_delayed_add_poll");
  void* complete = resolve("boltffi_async_method_class_rn_poc_counter_delayed_add_complete");
  void* free = resolve("boltffi_async_method_class_rn_poc_counter_delayed_add_free");

  auto trampoline = NativeContinuationTrampoline::create([](std::function<void()> fn) {
    // Off-thread hop simulation -- a real JSI adapter's ThreadHop is CallInvoker::invokeAsync;
    // here a detached thread stands in, exercising the SAME "never assume the calling thread"
    // discipline this file's own doc describes.
    std::thread([fn]() { fn(); }).detach();
  });

  std::atomic<bool> done{false};
  std::int32_t finalResult = -1;

  CValue entryArgs[2] = {CValue::ofU64(counter), CValue::ofU64(7)};
  std::uint64_t future = invokeGenericScalar(entry, entryArgs, 2);

  trampoline->registerContinuation(
      future,
      [poll](NativeHandle h, std::uint64_t callbackData, RawContinuationCallback cb) {
        CValue pollArgs[3] = {CValue::ofU64(h), CValue::ofU64(callbackData), CValue::ofPtr(reinterpret_cast<void*>(cb))};
        invokeGenericVoid(poll, pollArgs, 3);
      },
      [&](NativeHandle awaited) {
        std::uint8_t status[4] = {0, 0, 0, 0};
        CValue completeArgs[2] = {CValue::ofU64(awaited), CValue::ofPtr(status)};
        auto result = invokeGenericScalar(complete, completeArgs, 2);
        CValue freeArgs[1] = {CValue::ofU64(awaited)};
        invokeGenericVoid(free, freeArgs, 1);
        finalResult = static_cast<std::int32_t>(result);
        done = true;
      });

  for (int i = 0; i < 200 && !done; ++i) std::this_thread::sleep_for(std::chrono::milliseconds(10));
  BOLTFFI_CHECK(done.load());
  BOLTFFI_CHECK(finalResult == 27);  // 20 + 7
}

// ---- (B) generic callback vtable: Multiplier (ScalarReturn, synchronous) ----

BOLTFFI_TEST(generic_callback_vtable_drives_real_synchronous_multiplier) {
  VTableAbi vtable;
  vtable.name = "MultiplierVTable";
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"factor", TypeRef{PrimKind::I32}, {TypeRef{PrimKind::U64}}});

  for (auto& f : vtable.fields) BOLTFFI_CHECK(classifyVTableField(f).has_value());

  std::vector<void*> slots = buildVTableBytes(vtable);
  BOLTFFI_CHECK(slots.size() == 3);
  for (void* s : slots) BOLTFFI_CHECK(s != nullptr);

  void* registerFn = resolve("boltffi_register_callback_rn_poc_multiplier");
  CValue registerArgs[1] = {CValue::ofPtr(slots.data())};
  invokeGenericVoid(registerFn, registerArgs, 1);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["factor"] = [](const GenericMethodCall&, std::function<void(CallbackCompletion)>,
                                        CallbackResult& result) { result.scalar = static_cast<std::uint64_t>(3); };
  std::uint64_t callbackHandle = CallbackRegistry::instance().insert(registration);

  void* ctor = resolve("boltffi_init_class_rn_poc_counter_new");
  CValue ctorArgs[1] = {CValue::ofU64(5)};
  std::uint64_t counter = invokeGenericScalar(ctor, ctorArgs, 1);

  void* setMultiplier = resolve("boltffi_method_class_rn_poc_counter_set_multiplier");
  CValue setArgs[3] = {CValue::ofU64(counter), CValue::ofU64(callbackHandle), CValue::ofPtr(slots.data())};
  invokeGenericVoid(setMultiplier, setArgs, 3);

  void* scaled = resolve("boltffi_method_class_rn_poc_counter_scaled");
  CValue scaledArgs[1] = {CValue::ofU64(counter)};
  auto result = invokeGenericScalar(scaled, scaledArgs, 1);
  BOLTFFI_CHECK(static_cast<std::int32_t>(result) == 15);  // 5 * factor(3)
}

// ---- (B) generic callback vtable: AsyncKv::get (CompletionStatusBuf0, async, off-thread) ----
// The hardest real shape: Rust's OWN async method (`Counter::kv_get`) awaits a host-implemented
// async trait method (`AsyncKv::get`) whose completion fires later, from a background thread --
// exercising both async directions (Rust future continuation AND host-callback completion) in the
// same round trip, decoded through the real wire format (`Option<String>`: 1 tag byte, then --
// only if Some -- a 4-byte little-endian length prefix and the UTF-8 bytes, boltffi_core's
// `wire/encode.rs`).

BOLTFFI_TEST(generic_callback_vtable_drives_real_async_completion_kv_get) {
  VTableAbi vtable;
  vtable.name = "AsyncKvVTable";
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  TypeRef completionBuf;
  completionBuf.kind = PrimKind::FnPtr;
  completionBuf.fnPtrArity = 3;
  vtable.fields.push_back({"get", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}, completionBuf, TypeRef{PrimKind::PtrMut}}});

  for (auto& f : vtable.fields) BOLTFFI_CHECK(classifyVTableField(f).has_value());
  BOLTFFI_CHECK(*classifyVTableField(vtable.fields[2]) == CallbackShape::CompletionStatusBuf0);

  std::vector<void*> slots = buildVTableBytes(vtable);

  void* registerFn = resolve("boltffi_register_callback_rn_poc_async_kv");
  CValue registerArgs[1] = {CValue::ofPtr(slots.data())};
  invokeGenericVoid(registerFn, registerArgs, 1);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["get"] = [](const GenericMethodCall&, std::function<void(CallbackCompletion)> onComplete,
                                     CallbackResult&) {
    // Simulates a real async host operation (e.g. AsyncStorage.getItem()) completing later, from a
    // thread the JS/JSI call stack never touched -- the exact hazard CallInvoker::invokeAsync
    // exists to make safe on the real JSI side; this test drives the JSI-independent core directly.
    std::thread([onComplete]() {
      std::this_thread::sleep_for(std::chrono::milliseconds(30));
      const std::string value = "registered-value";
      std::vector<std::uint8_t> encoded;
      encoded.push_back(1);  // Option::Some tag
      std::uint32_t len = static_cast<std::uint32_t>(value.size());
      encoded.push_back(static_cast<std::uint8_t>(len & 0xff));
      encoded.push_back(static_cast<std::uint8_t>((len >> 8) & 0xff));
      encoded.push_back(static_cast<std::uint8_t>((len >> 16) & 0xff));
      encoded.push_back(static_cast<std::uint8_t>((len >> 24) & 0xff));
      encoded.insert(encoded.end(), value.begin(), value.end());
      onComplete(CallbackCompletion{0, std::move(encoded)});
    }).detach();
  };
  std::uint64_t callbackHandle = CallbackRegistry::instance().insert(registration);

  void* ctor = resolve("boltffi_init_class_rn_poc_counter_new");
  CValue ctorArgs[1] = {CValue::ofU64(1)};
  std::uint64_t counter = invokeGenericScalar(ctor, ctorArgs, 1);

  void* setKv = resolve("boltffi_method_class_rn_poc_counter_set_kv");
  CValue setKvArgs[3] = {CValue::ofU64(counter), CValue::ofU64(callbackHandle), CValue::ofPtr(slots.data())};
  invokeGenericVoid(setKv, setKvArgs, 3);

  void* entry = resolve("boltffi_method_class_rn_poc_counter_kv_get");
  void* poll = resolve("boltffi_async_method_class_rn_poc_counter_kv_get_poll");
  void* complete = resolve("boltffi_async_method_class_rn_poc_counter_kv_get_complete");
  void* free = resolve("boltffi_async_method_class_rn_poc_counter_kv_get_free");

  auto trampoline = NativeContinuationTrampoline::create(
      [](std::function<void()> fn) { std::thread([fn]() { fn(); }).detach(); });

  std::atomic<bool> done{false};
  std::vector<std::uint8_t> resultBuf;

  CValue entryArgs[1] = {CValue::ofU64(counter)};
  std::uint64_t future = invokeGenericScalar(entry, entryArgs, 1);

  trampoline->registerContinuation(
      future,
      [poll](NativeHandle h, std::uint64_t callbackData, RawContinuationCallback cb) {
        CValue pollArgs[3] = {CValue::ofU64(h), CValue::ofU64(callbackData), CValue::ofPtr(reinterpret_cast<void*>(cb))};
        invokeGenericVoid(poll, pollArgs, 3);
      },
      [&](NativeHandle awaited) {
        std::uint8_t status[4] = {0, 0, 0, 0};
        CValue completeArgs[2] = {CValue::ofU64(awaited), CValue::ofPtr(status)};
        struct FfiBufShape {
          std::uint8_t* ptr;
          std::uintptr_t len, cap, align;
        } out{};
        invokeGenericSret(complete, completeArgs, 2, &out, sizeof(out));
        CValue freeArgs[1] = {CValue::ofU64(awaited)};
        invokeGenericVoid(free, freeArgs, 1);

        std::int32_t statusCode = status[0] | (status[1] << 8) | (status[2] << 16) | (status[3] << 24);
        BOLTFFI_CHECK(statusCode == 0);
        resultBuf.assign(out.ptr, out.ptr + out.len);
        done = true;
      });

  for (int i = 0; i < 200 && !done; ++i) std::this_thread::sleep_for(std::chrono::milliseconds(10));
  BOLTFFI_CHECK(done.load());

  // Decode the real wire format: Option<String> -- tag byte, then a 4-byte LE length + UTF-8 bytes.
  BOLTFFI_CHECK(resultBuf.size() >= 5);
  BOLTFFI_CHECK(resultBuf[0] == 1);  // Some
  std::uint32_t len = resultBuf[1] | (resultBuf[2] << 8) | (resultBuf[3] << 16) | (resultBuf[4] << 24);
  BOLTFFI_CHECK(resultBuf.size() == 5 + len);
  std::string decoded(reinterpret_cast<const char*>(resultBuf.data() + 5), len);
  BOLTFFI_CHECK(decoded == "registered-value");
}

}  // namespace

int main() {
  const char* path = std::getenv("BOLTFFI_RN_POC_DYLIB");
  if (!path) {
    std::printf("BOLTFFI_RN_POC_DYLIB not set -- skipping real-crate integration tests\n");
    return 0;
  }
  gDylib = dlopen(path, RTLD_NOW);
  if (!gDylib) {
    std::printf("dlopen failed for %s: %s\n", path, dlerror());
    return 1;
  }
  return boltffi_test::runAll();
}
