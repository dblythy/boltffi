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

#include "boltffi/abi_header.h"
#include "test_harness.h"

using namespace boltffi;

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

BOLTFFI_TEST(unrecognized_shape_returns_nullopt_not_a_guess) {
  // `on_dropped`'s real shape (3 buffers + a scalar, sync void) -- documented as NOT YET covered.
  VTableFieldAbi onDropped{"on_dropped",
                           TypeRef{PrimKind::Void},
                           {TypeRef{PrimKind::U64}, TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64},
                            TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}, TypeRef{PrimKind::I32},
                            TypeRef{PrimKind::PtrConst}, TypeRef{PrimKind::U64}}};
  BOLTFFI_CHECK(!classifyVTableField(onDropped).has_value());
}

BOLTFFI_TEST(classifies_every_real_vtable_field_except_the_documented_gap) {
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
  // Exactly two documented gaps: EventuallyQueueListener::on_dropped's 3-buffer+scalar shape, and
  // RandomSource::fill's scalar-in/large-aggregate-out shape (see this file's module doc).
  BOLTFFI_CHECK(unclassified == 2);
}

BOLTFFI_TEST(exactly_the_two_documented_gaps_are_unclassified) {
  std::string path = std::string(BOLTFFI_TEST_SOURCE_DIR) + "/tests/fixtures/parse_core_real_abi.h";
  std::ifstream f(path);
  std::stringstream ss;
  ss << f.rdbuf();
  auto abi = parseAbiHeader(ss.str());

  bool foundOnDropped = false, foundFill = false;
  for (const auto& vtable : abi.vtables) {
    for (const auto& field : vtable.fields) {
      if (!classifyVTableField(field)) {
        std::string name = vtable.name + "::" + field.name;
        if (name == "___EventuallyQueueListenerVTable::on_dropped") foundOnDropped = true;
        if (name == "___RandomSourceVTable::fill") foundFill = true;
      }
    }
  }
  BOLTFFI_CHECK(foundOnDropped);
  BOLTFFI_CHECK(foundFill);
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

int main() { return boltffi_test::runAll(); }
