// Unit tests for the callback-vtable builder (boltffi/generic_callback.h). Pure C++, no jsi::
// dependency -- drives the generic trampolines directly as plain function pointers (standing in for
// what Rust would call), the same "JSI-independent core" discipline as native_trampoline_test.cpp.
// See real_crate_integration_test.cpp (gated on a real rn_poc build) for the complementary proof
// against genuine parse-core-family symbols (`Multiplier`, `AsyncKv`).
#include "boltffi/generic_callback.h"

#include <atomic>
#include <chrono>
#include <fstream>
#include <sstream>
#include <thread>

#include <cstring>

#include "boltffi/abi_header.h"
#include "test_harness.h"

using namespace boltffi;

// `trampolineScalarInBufOut` (the `fill`-shape trampoline) calls `makeFfiBufFromBytes`, which
// `dlsym`s a REAL `boltffi_buf_from_bytes` -- normally exported by whatever Rust dylib is loaded.
// This test binary never dlopens one (it exercises the JSI-independent core in isolation, like
// every other test in this file), so it provides its own spy -- same convention
// `ffi_buf_test.cpp` already uses for `boltffi_free_buf` (see that file's own header comment).
// Declared with the REAL 32-byte `FfiBuf_u8` field layout so it round-trips through
// `invokeGenericSret`'s aggregate-return convention exactly like the genuine Rust export would.
extern "C" {
struct SpyFfiBuf {
  std::uint8_t* ptr;
  std::uintptr_t len;
  std::uintptr_t cap;
  std::uintptr_t align;
};
SpyFfiBuf boltffi_buf_from_bytes(const std::uint8_t* ptr, std::uintptr_t len) {
  auto* copy = new std::uint8_t[len];
  if (len > 0) std::memcpy(copy, ptr, len);
  return SpyFfiBuf{copy, len, len, 1};
}
}

// `takeDeferredBuf`'s free half (`freeDeferredCallbackBytes`) `dlsym`s a REAL
// `boltffi_free_deferred_callback_bytes` -- same "this test binary provides its own spy" rationale
// as `boltffi_buf_from_bytes` above. Tracks every (ptr, len) pair it's asked to free so tests can
// assert the incoming-buffer leak (found by adversarial review of a91bde65) stays fixed: every
// trampoline that takes a deferred buffer parameter must free it exactly once, even when dispatch
// never finds a registered handle/method.
namespace {
std::vector<std::pair<void*, std::uintptr_t>>& deferredFrees() {
  static std::vector<std::pair<void*, std::uintptr_t>> frees;
  return frees;
}
}  // namespace

extern "C" {
void boltffi_free_deferred_callback_bytes(std::uint8_t* ptr, std::uintptr_t len) {
  deferredFrees().emplace_back(static_cast<void*>(ptr), len);
}
}

BOLTFFI_TEST(classifies_free_and_clone) {
  VTableFieldAbi freeField{"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}};
  VTableFieldAbi cloneField{"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}};
  BOLTFFI_CHECK(classifyVTableField(freeField) == CallbackShape::Free);
  BOLTFFI_CHECK(classifyVTableField(cloneField) == CallbackShape::Clone);
}

BOLTFFI_TEST(classifies_scalar_return_and_void_buf1) {
  VTableFieldAbi factor{"factor", TypeRef{PrimKind::I32}, {TypeRef{PrimKind::U64}}};
  BOLTFFI_CHECK(classifyVTableField(factor) == CallbackShape::ScalarReturn);

  VTableFieldAbi onEvent{"on_event",
                         TypeRef{PrimKind::Void},
                         {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}}};
  BOLTFFI_CHECK(classifyVTableField(onEvent) == CallbackShape::VoidBuf1);
}

BOLTFFI_TEST(classifies_completion_shapes_by_inner_fn_ptr_arity) {
  TypeRef statusOnly;
  statusOnly.kind = PrimKind::FnPtr;
  statusOnly.fnPtrArity = 2;
  TypeRef statusPlusBuf;
  statusPlusBuf.kind = PrimKind::FnPtr;
  statusPlusBuf.fnPtrArity = 3;

  VTableFieldAbi clear{"clear", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}, statusOnly, TypeRef{PrimKind::PtrMut}}};
  BOLTFFI_CHECK(classifyVTableField(clear) == CallbackShape::CompletionStatus0);

  VTableFieldAbi get{"get", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}, statusPlusBuf, TypeRef{PrimKind::PtrMut}}};
  BOLTFFI_CHECK(classifyVTableField(get) == CallbackShape::CompletionStatusBuf0);

  VTableFieldAbi set{"set",
                     TypeRef{PrimKind::Void},
                     {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}, statusOnly,
                      TypeRef{PrimKind::PtrMut}}};
  BOLTFFI_CHECK(classifyVTableField(set) == CallbackShape::CompletionStatus1);

  VTableFieldAbi kvGet{"get",
                       TypeRef{PrimKind::Void},
                       {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}, statusPlusBuf,
                        TypeRef{PrimKind::PtrMut}}};
  BOLTFFI_CHECK(classifyVTableField(kvGet) == CallbackShape::CompletionStatusBuf1);
}

BOLTFFI_TEST(classifies_on_dropped_and_fill_the_former_gaps) {
  // Finding 4, this session: these two shapes used to return `nullopt` (and `buildVTableBytes`
  // silently installed a NULL vtable slot for them) -- now covered as `VoidBuf2Scalar1Buf1`/
  // `ScalarInBufOut`.
  VTableFieldAbi onDropped{"on_dropped",
                           TypeRef{PrimKind::Void},
                           {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64},
                            TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}, TypeRef{PrimKind::I32},
                            TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}}};
  BOLTFFI_CHECK(classifyVTableField(onDropped) == CallbackShape::VoidBuf2Scalar1Buf1);

  TypeRef ffiBufReturn;
  ffiBufReturn.kind = PrimKind::Aggregate;
  ffiBufReturn.aggregateName = "FfiBuf_u8";
  VTableFieldAbi fill{"fill", ffiBufReturn, {TypeRef{PrimKind::U64}, TypeRef{PrimKind::U32}}};
  BOLTFFI_CHECK(classifyVTableField(fill) == CallbackShape::ScalarInBufOut);
}

BOLTFFI_TEST(classifies_every_real_vtable_field_13_of_13) {
  std::string path = std::string(BOLTFFI_TEST_SOURCE_DIR) + "/tests/fixtures/parse_core_real_abi.h";
  std::ifstream f(path);
  std::stringstream ss;
  ss << f.rdbuf();
  auto abi = parseAbiHeader(ss.str());

  int unclassified = 0;
  std::string unclassifiedName;
  for (const auto& vtable : abi.vtables) {
    for (const auto& field : vtable.fields) {
      if (!classifyVTableField(field)) {
        ++unclassified;
        unclassifiedName = vtable.name + "::" + field.name;
      }
    }
  }
  // Finding 4's fix closes both former gaps (EventuallyQueueListener::on_dropped,
  // RandomSource::fill) -- every real vtable field now classifies.
  BOLTFFI_CHECK(unclassified == 0);
}

BOLTFFI_TEST(void_buf2_scalar1_buf1_trampoline_round_trip) {
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"on_dropped",
                            TypeRef{PrimKind::Void},
                            {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64},
                             TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}, TypeRef{PrimKind::I32},
                             TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}}});
  auto slots = buildVTableBytes(vtable);
  BOLTFFI_CHECK(slots.size() == 3);
  for (void* s : slots) BOLTFFI_CHECK(s != nullptr);

  std::vector<std::uint8_t> seenBuf0, seenBuf1, seenBuf2;
  std::int32_t seenScalar = -1;
  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["on_dropped"] = [&](const GenericMethodCall& call, std::function<void(CallbackCompletion)>,
                                             CallbackResult&) {
    seenBuf0 = call.bufArgs[0];
    seenBuf1 = call.bufArgs[1];
    seenBuf2 = call.bufArgs[2];
    seenScalar = static_cast<std::int32_t>(call.scalarArg);
  };
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  auto onDroppedFn = reinterpret_cast<void (*)(std::uint64_t, const std::uint8_t*, std::uintptr_t,
                                                const std::uint8_t*, std::uintptr_t, std::int32_t,
                                                const std::uint8_t*, std::uintptr_t)>(slots[2]);
  std::uint8_t buf0[] = {1, 2, 3};
  std::uint8_t buf1[] = {4, 5};
  std::uint8_t buf2[] = {6};
  onDroppedFn(handle, buf0, sizeof(buf0), buf1, sizeof(buf1), 42, buf2, sizeof(buf2));

  BOLTFFI_CHECK((seenBuf0 == std::vector<std::uint8_t>{1, 2, 3}));
  BOLTFFI_CHECK((seenBuf1 == std::vector<std::uint8_t>{4, 5}));
  BOLTFFI_CHECK((seenBuf2 == std::vector<std::uint8_t>{6}));
  BOLTFFI_CHECK(seenScalar == 42);

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slots[0]);
  freeFn(handle);
}

BOLTFFI_TEST(scalar_in_buf_out_trampoline_round_trip) {
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  TypeRef ffiBufReturn;
  ffiBufReturn.kind = PrimKind::Aggregate;
  ffiBufReturn.aggregateName = "FfiBuf_u8";
  vtable.fields.push_back({"fill", ffiBufReturn, {TypeRef{PrimKind::U64}, TypeRef{PrimKind::U32}}});
  auto slots = buildVTableBytes(vtable);
  BOLTFFI_CHECK(slots.size() == 3);
  for (void* s : slots) BOLTFFI_CHECK(s != nullptr);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["fill"] = [](const GenericMethodCall& call, std::function<void(CallbackCompletion)>,
                                      CallbackResult& result) {
    BOLTFFI_CHECK(call.hasScalarArg);
    result.bytes.assign(call.scalarArg, static_cast<std::uint8_t>(0xAB));
  };
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  auto fillFn = reinterpret_cast<detail::FfiBufReturn (*)(std::uint64_t, std::uint32_t)>(slots[2]);
  detail::FfiBufReturn out = fillFn(handle, 5);
  BOLTFFI_CHECK(out.len == 5);
  BOLTFFI_CHECK(out.ptr != nullptr);
  for (std::uintptr_t i = 0; i < out.len; ++i) BOLTFFI_CHECK(out.ptr[i] == 0xAB);

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slots[0]);
  freeFn(handle);
}

BOLTFFI_TEST(scalar_return_trampoline_round_trip) {
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"factor", TypeRef{PrimKind::I32}, {TypeRef{PrimKind::U64}}});
  auto slots = buildVTableBytes(vtable);
  BOLTFFI_CHECK(slots.size() == 3);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["factor"] = [](const GenericMethodCall&, std::function<void(CallbackCompletion)>,
                                        CallbackResult& result) { result.scalar = 7; };
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  auto factorFn = reinterpret_cast<std::uint64_t (*)(std::uint64_t)>(slots[2]);
  BOLTFFI_CHECK(factorFn(handle) == 7);

  // clone shares the SAME registration (a real ref-count clone, not a copy).
  auto cloneFn = reinterpret_cast<std::uint64_t (*)(std::uint64_t)>(slots[1]);
  std::uint64_t cloned = cloneFn(handle);
  BOLTFFI_CHECK(cloned != 0 && cloned != handle);
  BOLTFFI_CHECK(factorFn(cloned) == 7);

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slots[0]);
  freeFn(handle);
  freeFn(cloned);
  BOLTFFI_CHECK(CallbackRegistry::instance().find(handle) == nullptr);
}

BOLTFFI_TEST(completion_status0_trampoline_delivers_off_thread) {
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  TypeRef statusOnly;
  statusOnly.kind = PrimKind::FnPtr;
  statusOnly.fnPtrArity = 2;
  vtable.fields.push_back({"clear", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}, statusOnly, TypeRef{PrimKind::PtrMut}}});
  auto slots = buildVTableBytes(vtable);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["clear"] = [](const GenericMethodCall&, std::function<void(CallbackCompletion)> onComplete,
                                       CallbackResult&) {
    std::thread([onComplete]() {
      std::this_thread::sleep_for(std::chrono::milliseconds(20));
      onComplete(CallbackCompletion{0, {}});
    }).detach();
  };
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  std::atomic<int32_t> received{-1};
  auto clearFn = reinterpret_cast<void (*)(std::uint64_t, detail::CompletionStatusFn, void*)>(slots[2]);
  auto completion = +[](void* ud, std::int32_t status) { *reinterpret_cast<std::atomic<int32_t>*>(ud) = status; };
  clearFn(handle, completion, &received);

  for (int i = 0; i < 100 && received.load() == -1; ++i) std::this_thread::sleep_for(std::chrono::milliseconds(10));
  BOLTFFI_CHECK(received.load() == 0);

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slots[0]);
  freeFn(handle);
}

BOLTFFI_TEST(missing_registration_reports_cancelled_not_ok) {
  // Finding 7, this session: a completion trampoline racing teardown (the registered object was
  // already freed, or the method was never registered) used to call back with a
  // default-constructed `CallbackCompletion{}` -- `statusCode == 0`, `FFI_STATUS_OK` -- reporting
  // success for an operation that never ran. It must report `FFI_STATUS_CANCELLED` (4) instead.
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  TypeRef statusOnly;
  statusOnly.kind = PrimKind::FnPtr;
  statusOnly.fnPtrArity = 2;
  vtable.fields.push_back(
      {"clear2", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}, statusOnly, TypeRef{PrimKind::PtrMut}}});
  auto slots = buildVTableBytes(vtable);

  std::atomic<int32_t> received{-1};
  auto clearFn = reinterpret_cast<void (*)(std::uint64_t, detail::CompletionStatusFn, void*)>(slots[2]);
  auto completion = +[](void* ud, std::int32_t status) { *reinterpret_cast<std::atomic<int32_t>*>(ud) = status; };
  // NEVER inserted into CallbackRegistry -- handle 0xDEAD is guaranteed not found.
  clearFn(0xDEADULL, completion, &received);
  BOLTFFI_CHECK(received.load() == kFfiStatusCancelled);
}

BOLTFFI_TEST(slot_allocation_is_cached_per_shape_and_method_not_exhausted_by_reregistration) {
  // Finding 6, this session: registering the SAME (shape, method name) pair repeatedly (e.g.
  // multiple `ParseClient` instances installing the same `SessionStorage`-shaped trait, or a hot
  // reload) used to burn a fresh SlotId from the fixed `kMaxCallbackSlots` budget every single
  // time, exhausting it under nothing more exotic than ordinary repeated use. Re-registering it
  // far more than `kMaxCallbackSlots` times must never throw, and must always yield the identical
  // trampoline pointer.
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"factor", TypeRef{PrimKind::I32}, {TypeRef{PrimKind::U64}}});

  void* first = buildCallbackTrampoline(CallbackShape::ScalarReturn, "factor");
  for (int i = 0; i < 4 * static_cast<int>(kMaxCallbackSlots); ++i) {
    void* again = buildCallbackTrampoline(CallbackShape::ScalarReturn, "factor");
    BOLTFFI_CHECK(again == first);
  }

  // A DIFFERENT method name under the same shape must still get its OWN (cached) slot.
  void* other = buildCallbackTrampoline(CallbackShape::ScalarReturn, "now_ms");
  BOLTFFI_CHECK(other != first);
  void* otherAgain = buildCallbackTrampoline(CallbackShape::ScalarReturn, "now_ms");
  BOLTFFI_CHECK(otherAgain == other);
}

BOLTFFI_TEST(registered_vtable_survives_the_registering_functions_return) {
  // Finding 5, this session: the real generated `boltffi_register_callback_*` stores the raw
  // pointer it's given in a process-wide `static AtomicPtr` and dereferences it on every
  // subsequent trait call FOREVER (verified against the actual codegen,
  // `boltffi_macros/src/experimental/wrapper/callback.rs`'s `#register_ident`) -- there is no copy
  // and no teardown call. `registerVTableForProcessLifetime` must hand back a pointer that stays
  // valid long after the function that built the `VTableAbi` returns (a plain
  // `buildVTableBytes(vtable)` local variable would NOT survive this).
  auto buildAndRegister = []() -> const void* {
    VTableAbi vtable;
    vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
    vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
    vtable.fields.push_back({"factor", TypeRef{PrimKind::I32}, {TypeRef{PrimKind::U64}}});
    return registerVTableForProcessLifetime(vtable);
    // `vtable` (the input VTableAbi) goes out of scope here -- irrelevant, since
    // `registerVTableForProcessLifetime` only reads it to build its OWN heap-owned copy.
  };
  const void* vtablePtr = buildAndRegister();

  // Encourage stack/heap reuse that would surface a dangling pointer if the storage were NOT
  // process-lifetime (a plain local `std::vector` returned by address would be a textbook UAF
  // here, likely to get its memory reused by the allocations below).
  std::vector<std::vector<int>> churn;
  for (int i = 0; i < 64; ++i) churn.emplace_back(64, i);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["factor"] = [](const GenericMethodCall&, std::function<void(CallbackCompletion)>,
                                        CallbackResult& result) { result.scalar = 9; };
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  const auto* slotArray = reinterpret_cast<void* const*>(vtablePtr);
  auto factorFn = reinterpret_cast<std::uint64_t (*)(std::uint64_t)>(slotArray[2]);
  BOLTFFI_CHECK(factorFn(handle) == 9);

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slotArray[0]);
  freeFn(handle);
}

BOLTFFI_TEST(void_buf1_trampoline_frees_the_transferred_buffer_exactly_once) {
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"on_event",
                            TypeRef{PrimKind::Void},
                            {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}}});
  auto slots = buildVTableBytes(vtable);

  std::vector<std::uint8_t> seen;
  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["on_event"] = [&](const GenericMethodCall& call, std::function<void(CallbackCompletion)>,
                                           CallbackResult&) { seen = call.bufArgs[0]; };
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  deferredFrees().clear();
  std::uint8_t buf[] = {9, 8, 7};
  auto onEventFn =
      reinterpret_cast<void (*)(std::uint64_t, const std::uint8_t*, std::uintptr_t)>(slots[2]);
  onEventFn(handle, buf, sizeof(buf));

  BOLTFFI_CHECK((seen == std::vector<std::uint8_t>{9, 8, 7}));
  BOLTFFI_CHECK(deferredFrees().size() == 1);
  BOLTFFI_CHECK(deferredFrees()[0].first == static_cast<void*>(buf));
  BOLTFFI_CHECK(deferredFrees()[0].second == sizeof(buf));

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slots[0]);
  freeFn(handle);
}

BOLTFFI_TEST(void_buf1_trampoline_frees_the_transferred_buffer_even_when_handle_is_unknown) {
  // The leak's worst case (adversarial finding): a call that can't be dispatched at all -- unknown
  // handle -- must still return ownership of the transferred bytes rather than leaking them.
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"on_event",
                            TypeRef{PrimKind::Void},
                            {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}}});
  auto slots = buildVTableBytes(vtable);

  deferredFrees().clear();
  std::uint8_t buf[] = {1, 2};
  auto onEventFn =
      reinterpret_cast<void (*)(std::uint64_t, const std::uint8_t*, std::uintptr_t)>(slots[2]);
  onEventFn(0xDEADBEEFULL, buf, sizeof(buf));  // never registered

  BOLTFFI_CHECK(deferredFrees().size() == 1);
  BOLTFFI_CHECK(deferredFrees()[0].first == static_cast<void*>(buf));
  BOLTFFI_CHECK(deferredFrees()[0].second == sizeof(buf));
}

BOLTFFI_TEST(void_buf2_scalar1_buf1_trampoline_frees_all_three_transferred_buffers) {
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"on_dropped",
                            TypeRef{PrimKind::Void},
                            {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64},
                             TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}, TypeRef{PrimKind::I32},
                             TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}}});
  auto slots = buildVTableBytes(vtable);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["on_dropped"] = [](const GenericMethodCall&, std::function<void(CallbackCompletion)>,
                                            CallbackResult&) {};
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  deferredFrees().clear();
  std::uint8_t buf0[] = {1}, buf1[] = {2, 3}, buf2[] = {4, 5, 6};
  auto onDroppedFn = reinterpret_cast<void (*)(std::uint64_t, const std::uint8_t*, std::uintptr_t,
                                                const std::uint8_t*, std::uintptr_t, std::int32_t,
                                                const std::uint8_t*, std::uintptr_t)>(slots[2]);
  onDroppedFn(handle, buf0, sizeof(buf0), buf1, sizeof(buf1), 0, buf2, sizeof(buf2));

  BOLTFFI_CHECK(deferredFrees().size() == 3);

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slots[0]);
  freeFn(handle);
}

BOLTFFI_TEST(completion_status1_trampoline_frees_the_transferred_buffer) {
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  TypeRef statusOnly;
  statusOnly.kind = PrimKind::FnPtr;
  statusOnly.fnPtrArity = 2;
  vtable.fields.push_back({"set",
                            TypeRef{PrimKind::Void},
                            {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64},
                             statusOnly, TypeRef{PrimKind::PtrMut}}});
  auto slots = buildVTableBytes(vtable);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["set"] = [](const GenericMethodCall&, std::function<void(CallbackCompletion)> onComplete,
                                     CallbackResult&) { onComplete(CallbackCompletion{0, {}}); };
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  deferredFrees().clear();
  std::uint8_t buf[] = {1, 2, 3, 4};
  std::atomic<int32_t> received{-1};
  auto setFn = reinterpret_cast<void (*)(std::uint64_t, const std::uint8_t*, std::uintptr_t,
                                          detail::CompletionStatusFn, void*)>(slots[2]);
  auto completion = +[](void* ud, std::int32_t status) { *reinterpret_cast<std::atomic<int32_t>*>(ud) = status; };
  setFn(handle, buf, sizeof(buf), completion, &received);

  BOLTFFI_CHECK(received.load() == 0);
  BOLTFFI_CHECK(deferredFrees().size() == 1);
  BOLTFFI_CHECK(deferredFrees()[0].second == sizeof(buf));

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slots[0]);
  freeFn(handle);
}

BOLTFFI_TEST(completion_status_buf1_trampoline_frees_the_transferred_buffer) {
  VTableAbi vtable;
  vtable.fields.push_back({"free", TypeRef{PrimKind::Void}, {TypeRef{PrimKind::U64}}});
  vtable.fields.push_back({"clone", TypeRef{PrimKind::U64}, {TypeRef{PrimKind::U64}}});
  TypeRef statusPlusBuf;
  statusPlusBuf.kind = PrimKind::FnPtr;
  statusPlusBuf.fnPtrArity = 3;
  vtable.fields.push_back({"get",
                            TypeRef{PrimKind::Void},
                            {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64},
                             statusPlusBuf, TypeRef{PrimKind::PtrMut}}});
  auto slots = buildVTableBytes(vtable);

  auto registration = std::make_shared<RegisteredCallbackObject>();
  registration->methods["get"] = [](const GenericMethodCall&, std::function<void(CallbackCompletion)> onComplete,
                                     CallbackResult&) { onComplete(CallbackCompletion{0, {42}}); };
  std::uint64_t handle = CallbackRegistry::instance().insert(registration);

  deferredFrees().clear();
  std::uint8_t buf[] = {1, 2, 3};
  std::atomic<bool> done{false};
  auto getFn = reinterpret_cast<void (*)(std::uint64_t, const std::uint8_t*, std::uintptr_t,
                                          detail::CompletionStatusBufFn, void*)>(slots[2]);
  auto completion = +[](void* ud, std::int32_t, LargeAggregate) { *reinterpret_cast<std::atomic<bool>*>(ud) = true; };
  getFn(handle, buf, sizeof(buf), completion, &done);

  BOLTFFI_CHECK(done.load());
  BOLTFFI_CHECK(deferredFrees().size() == 1);
  BOLTFFI_CHECK(deferredFrees()[0].second == sizeof(buf));

  auto freeFn = reinterpret_cast<void (*)(std::uint64_t)>(slots[0]);
  freeFn(handle);
}

int main() { return boltffi_test::runAll(); }
