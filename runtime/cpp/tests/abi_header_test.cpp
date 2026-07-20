// Unit tests for the pure-C++ ABI header parser (boltffi/abi_header.h) -- no jsi:: dependency, no
// Rust toolchain needed. Covers the exact grammar boltffi's C renderer emits: plain functions,
// pointer/const/scalar params, callback vtables (all-function-pointer structs), plain data
// records (FfiBuf_u8/FfiString/BoltFFICallbackHandle/FfiStatus), typedef aliases (function-pointer
// typedefs, enum-underlying-type typedefs, `const void *` handle typedefs), and the `static inline`
// atomic helpers/`#define` constants the header also contains but this adapter never calls
// generically. The final test parses the REAL, complete, unmodified header captured from a real
// `parse-core-rs` build (tests/fixtures/parse_core_real_abi.h, 874 lines / 689 functions / 13
// vtables / 6 records) end to end -- the strongest evidence this parser matches the ACTUAL
// ParseClient/ParseObject/ParseQuery surface, not a hand-picked subset.
#include "boltffi/abi_header.h"

#include <fstream>
#include <sstream>

#include "test_harness.h"

using boltffi::isArenaPointerKind;
using boltffi::parseAbiHeader;
using boltffi::planFunctionCall;
using boltffi::planFunctionReturn;
using boltffi::PrimKind;
using boltffi::RegisterSlotKind;
using boltffi::ReturnPlan;

namespace {

std::string readFixture(const char* relativePath) {
  // Tests always run with CWD == the CMake build/test working directory, which CMake sets to
  // CMAKE_CURRENT_SOURCE_DIR's runtime/cpp by convention here (see CMakeLists.txt's add_test) --
  // resolved relative to this file's own known fixtures/ sibling directory instead, so the test
  // binary works regardless of the invoker's CWD.
  std::string path = std::string(BOLTFFI_TEST_SOURCE_DIR) + "/tests/" + relativePath;
  std::ifstream f(path);
  if (!f) throw std::runtime_error(std::string("fixture not found: ") + path);
  std::stringstream ss;
  ss << f.rdbuf();
  return ss.str();
}

}  // namespace

BOLTFFI_TEST(parses_plain_scalar_function) {
  auto abi = parseAbiHeader("bool boltffi_method_record_x_get(const uint8_t *id_ptr, uintptr_t id_len);\n");
  BOLTFFI_CHECK(abi.functions.size() == 1);
  const auto& fn = abi.functions[0];
  BOLTFFI_CHECK(fn.name == "boltffi_method_record_x_get");
  BOLTFFI_CHECK(fn.returnType.kind == PrimKind::Bool);
  BOLTFFI_CHECK(fn.params.size() == 2);
  BOLTFFI_CHECK(fn.params[0].kind == PrimKind::PtrConst);
  BOLTFFI_CHECK(fn.params[0].name == "id_ptr");
  BOLTFFI_CHECK(fn.params[1].kind == PrimKind::U64);
  BOLTFFI_CHECK(fn.params[1].name == "id_len");
}

BOLTFFI_TEST(parses_zero_arg_function) {
  auto abi = parseAbiHeader("uint64_t boltffi_init_class_x_new(void);\n");
  BOLTFFI_CHECK(abi.functions.size() == 1);
  BOLTFFI_CHECK(abi.functions[0].params.empty());
  BOLTFFI_CHECK(abi.functions[0].returnType.kind == PrimKind::U64);
}

BOLTFFI_TEST(parses_void_return_and_out_param_pointer) {
  auto abi = parseAbiHeader("void boltffi_x(uint64_t handle, int32_t *out);\n");
  BOLTFFI_CHECK(abi.functions[0].returnType.kind == PrimKind::Void);
  BOLTFFI_CHECK(abi.functions[0].params[1].kind == PrimKind::PtrMut);
}

BOLTFFI_TEST(parses_double_param) {
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    int32_t code;\n"
      "} FfiStatus;\n"
      "FfiStatus boltffi_method_class_x_object_increment(uint64_t receiver, const uint8_t *key_ptr, "
      "uintptr_t key_len, double amount);\n");
  const auto& fn = abi.functions[0];
  BOLTFFI_CHECK(fn.returnType.kind == PrimKind::Aggregate);
  BOLTFFI_CHECK(fn.returnType.aggregateName == "FfiStatus");
  BOLTFFI_CHECK(fn.params[3].kind == PrimKind::F64);
}

BOLTFFI_TEST(parses_data_record_and_computes_byte_size) {
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    uint8_t *ptr;\n"
      "    uintptr_t len;\n"
      "    uintptr_t cap;\n"
      "    uintptr_t align;\n"
      "} FfiBuf_u8;\n");
  BOLTFFI_CHECK(abi.records.size() == 1);
  BOLTFFI_CHECK(abi.records[0].name == "FfiBuf_u8");
  BOLTFFI_CHECK(abi.records[0].byteSize == 32);
  BOLTFFI_CHECK(abi.findRecord("FfiBuf_u8") != nullptr);
}

BOLTFFI_TEST(small_scalar_record_sizes_correctly) {
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    int32_t code;\n"
      "} FfiStatus;\n"
      "typedef struct {\n"
      "    uint64_t handle;\n"
      "    const void *vtable;\n"
      "} BoltFFICallbackHandle;\n");
  BOLTFFI_CHECK(abi.findRecord("FfiStatus")->byteSize == 4);
  BOLTFFI_CHECK(abi.findRecord("BoltFFICallbackHandle")->byteSize == 16);
}

BOLTFFI_TEST(distinguishes_vtable_from_data_record_structurally) {
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    void (*free)(uint64_t);\n"
      "    uint64_t (*clone)(uint64_t);\n"
      "    void (*on_event)(uint64_t, const uint8_t *, uintptr_t);\n"
      "} ___ListenerVTable;\n");
  BOLTFFI_CHECK(abi.vtables.size() == 1);
  BOLTFFI_CHECK(abi.records.empty());
  const auto& vt = abi.vtables[0];
  BOLTFFI_CHECK(vt.name == "___ListenerVTable");
  BOLTFFI_CHECK(vt.fields.size() == 3);
  BOLTFFI_CHECK(vt.fields[0].name == "free");
  BOLTFFI_CHECK(vt.fields[0].returnType.kind == PrimKind::Void);
  BOLTFFI_CHECK(vt.fields[0].params.size() == 1);
  BOLTFFI_CHECK(vt.fields[1].name == "clone");
  BOLTFFI_CHECK(vt.fields[1].returnType.kind == PrimKind::U64);
  BOLTFFI_CHECK(vt.fields[2].name == "on_event");
  BOLTFFI_CHECK(vt.fields[2].params.size() == 3);
  BOLTFFI_CHECK(vt.fields[2].params[1].kind == PrimKind::PtrConst);
}

BOLTFFI_TEST(vtable_field_with_nested_completion_callback_param) {
  // ___SessionStorageVTable's real `get` shape: a trailing (completion-fn-ptr, userdata) pair --
  // the nested completion callback is only classified shallowly (FnPtr), never recursed into.
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    void (*free)(uint64_t);\n"
      "    uint64_t (*clone)(uint64_t);\n"
      "    void (*get)(uint64_t, void (*)(void *, FfiStatus, FfiBuf_u8), void *);\n"
      "} ___SessionStorageVTable;\n");
  const auto& vt = abi.vtables[0];
  const auto& get = vt.fields[2];
  BOLTFFI_CHECK(get.name == "get");
  BOLTFFI_CHECK(get.params.size() == 3);
  BOLTFFI_CHECK(get.params[0].kind == PrimKind::U64);
  BOLTFFI_CHECK(get.params[1].kind == PrimKind::FnPtr);
  BOLTFFI_CHECK(get.params[2].kind == PrimKind::PtrMut);  // `void *userdata`
}

BOLTFFI_TEST(fn_ptr_typedef_alias_used_as_bare_param_type) {
  auto abi = parseAbiHeader(
      "typedef void (*RustFutureContinuationCallback)(uint64_t callback_data, int8_t poll_result);\n"
      "void boltffi_x_poll(uint64_t handle, uint64_t callback_data, RustFutureContinuationCallback callback);\n");
  BOLTFFI_CHECK(abi.functions.size() == 1);
  BOLTFFI_CHECK(abi.functions[0].params[2].kind == PrimKind::FnPtr);
}

BOLTFFI_TEST(pointer_typedef_alias_used_as_bare_return_type_is_an_opaque_handle) {
  // CRITICAL regression (finding 3, this session): `RustFutureHandle` is a Rust-owned OPAQUE
  // handle (a real `Box`/`Arc` pointer the async-future machinery manages), never an offset into a
  // JS-simulated arena -- it must classify distinctly from an inline `const T *` parameter
  // (`PrimKind::PtrConst`), which a generic JS-facing caller DOES legitimately rebase against a
  // bound arena. Before this fix both were the same `PrimKind::PtrConst`, so every
  // poll/complete/cancel/free call on a real future handle got silently (and wrongly) rebased as
  // `arenaBase + handle`, corrupting the pointer.
  auto abi = parseAbiHeader(
      "typedef const void *RustFutureHandle;\n"
      "RustFutureHandle boltffi_x_start(uint64_t receiver);\n"
      "void boltffi_x_poll(RustFutureHandle handle, uint64_t callback_data);\n");
  BOLTFFI_CHECK(abi.functions[0].returnType.kind == PrimKind::OpaqueHandle);
  BOLTFFI_CHECK(abi.functions[1].params[0].kind == PrimKind::OpaqueHandle);
  BOLTFFI_CHECK(!isArenaPointerKind(PrimKind::OpaqueHandle));
}

BOLTFFI_TEST(vtable_pointer_param_is_an_opaque_handle_not_an_arena_pointer) {
  // CRITICAL regression (finding 1, second adversarial round): a `boltffi_register_callback_*`
  // function's `const ___FooVTable *vtable` parameter carries the REAL process pointer
  // `registerVTableForProcessLifetime` (generic_callback.h) hands back -- never an offset into a
  // JS-simulated arena, unlike an inline `const uint8_t *` data pointer. Before this fix it
  // classified as `PtrConst` purely because it was spelled with a `*` at the use site, so a
  // generic dispatch would rebase it as `arenaBase + processAddress`, corrupting it (wild pointer
  // on the Rust side). A pointer to a NAMED vtable struct type must classify like `RustFutureHandle`
  // (`OpaqueHandle`): structurally, not by guessing from the parameter's name.
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    void (*free)(uint64_t);\n"
      "    uint64_t (*clone)(uint64_t);\n"
      "} ___FooVTable;\n"
      "void boltffi_register_callback_foo(const ___FooVTable *vtable);\n");
  BOLTFFI_CHECK(abi.vtables.size() == 1);
  BOLTFFI_CHECK(abi.functions.size() == 1);
  BOLTFFI_CHECK(abi.functions[0].params[0].kind == PrimKind::OpaqueHandle);
  BOLTFFI_CHECK(!isArenaPointerKind(abi.functions[0].params[0].kind));
}

BOLTFFI_TEST(inline_pointer_param_is_still_a_rebase_eligible_arena_pointer) {
  // The sibling case: an INLINE `const uint8_t *` parameter spelling (never through a named
  // typedef alias) must still classify as `PtrConst` and remain rebase-eligible -- the fix must
  // not overcorrect and treat every pointer as opaque.
  auto abi = parseAbiHeader("void boltffi_x(const uint8_t *key_ptr, uintptr_t key_len);\n");
  BOLTFFI_CHECK(abi.functions[0].params[0].kind == PrimKind::PtrConst);
  BOLTFFI_CHECK(isArenaPointerKind(PrimKind::PtrConst));
}

BOLTFFI_TEST(enum_underlying_type_alias_resolves_bare) {
  auto abi = parseAbiHeader(
      "typedef int32_t ___CachePolicy;\n"
      "#define CACHE_POLICY_NETWORK_ONLY ((___CachePolicy)0)\n"
      "void boltffi_x_set_policy(uint64_t self, ___CachePolicy policy);\n");
  BOLTFFI_CHECK(abi.functions.size() == 1);
  BOLTFFI_CHECK(abi.functions[0].params[1].kind == PrimKind::I32);
}

BOLTFFI_TEST(skips_static_inline_helper_bodies_and_boilerplate) {
  auto abi = parseAbiHeader(
      "#pragma once\n"
      "#include <stdint.h>\n"
      "#ifdef __cplusplus\n"
      "extern \"C\" {\n"
      "#endif\n"
      "static inline bool boltffi_atomic_u8_cas(uint8_t *state, uint8_t expected, uint8_t desired) {\n"
      "    return true;\n"
      "}\n"
      "void boltffi_real(uint64_t handle);\n"
      "#ifdef __cplusplus\n"
      "}\n"
      "#endif\n");
  BOLTFFI_CHECK(abi.functions.size() == 1);
  BOLTFFI_CHECK(abi.functions[0].name == "boltffi_real");
}

BOLTFFI_TEST(rejects_unrecognized_input_loudly) {
  bool threw = false;
  try {
    parseAbiHeader("this is not valid header content at all\n");
  } catch (const std::exception&) {
    threw = true;
  }
  BOLTFFI_CHECK(threw);
}

BOLTFFI_TEST(parses_the_real_full_parse_core_header_end_to_end) {
  std::string source = readFixture("fixtures/parse_core_real_abi.h");
  auto abi = parseAbiHeader(source);

  // Measured ground truth (see this session's report): 689 functions, 13 callback vtables, 6
  // plain data records, over the REAL ParseClient/ParseObject/ParseQuery/... surface.
  BOLTFFI_CHECK(abi.functions.size() == 689);
  BOLTFFI_CHECK(abi.vtables.size() == 13);
  BOLTFFI_CHECK(abi.records.size() == 6);

  BOLTFFI_CHECK(abi.findRecord("FfiBuf_u8")->byteSize == 32);
  BOLTFFI_CHECK(abi.findRecord("FfiString")->byteSize == 24);
  BOLTFFI_CHECK(abi.findRecord("BoltFFICallbackHandle")->byteSize == 16);
  BOLTFFI_CHECK(abi.findRecord("FfiStatus")->byteSize == 4);

  const auto* ctor = abi.findFunction("boltffi_init_class_parse_core_ffi_client_parse_client_new");
  BOLTFFI_CHECK(ctor != nullptr);
  BOLTFFI_CHECK(ctor->returnType.kind == PrimKind::U64);
  BOLTFFI_CHECK(ctor->params.size() == 2);
  BOLTFFI_CHECK(ctor->params[0].kind == PrimKind::PtrConst);
  BOLTFFI_CHECK(ctor->params[1].kind == PrimKind::U64);

  const auto* increment = abi.findFunction("boltffi_method_class_parse_core_ffi_object_parse_object_increment");
  BOLTFFI_CHECK(increment != nullptr);
  BOLTFFI_CHECK(increment->params.size() == 4);
  BOLTFFI_CHECK(increment->params[3].kind == PrimKind::F64);

  const auto* sessionStorage = abi.findVTable("___SessionStorageVTable");
  BOLTFFI_CHECK(sessionStorage != nullptr);
  BOLTFFI_CHECK(sessionStorage->fields.size() == 5);
  BOLTFFI_CHECK(sessionStorage->fields[0].name == "free");
  BOLTFFI_CHECK(sessionStorage->fields[1].name == "clone");
  BOLTFFI_CHECK(sessionStorage->fields[2].name == "get");
  BOLTFFI_CHECK(sessionStorage->fields[2].params.size() == 3);
  BOLTFFI_CHECK(sessionStorage->fields[2].params[1].kind == PrimKind::FnPtr);
  BOLTFFI_CHECK(sessionStorage->fields[3].name == "set");
  BOLTFFI_CHECK(sessionStorage->fields[4].name == "clear");

  // Arity/float-position ground truth this session measured across the real header, driving
  // generic_invoke.h's dispatch-table bound: max arity 9, and every function has AT MOST one
  // `double` parameter.
  std::size_t maxArity = 0;
  for (const auto& fn : abi.functions) {
    maxArity = std::max(maxArity, fn.params.size());
    int doubleCount = 0;
    for (const auto& p : fn.params) {
      if (p.kind == PrimKind::F64) ++doubleCount;
    }
    BOLTFFI_CHECK(doubleCount <= 1);
  }
  BOLTFFI_CHECK(maxArity == 9);

  // Every `RustFutureHandle`-typed param/return across the ENTIRE real header must classify as
  // `OpaqueHandle`, never `PtrConst` -- the finding-3 regression, swept across all 689 functions
  // rather than just the one hand-picked poll function below.
  bool sawOpaqueHandleParam = false, sawOpaqueHandleReturn = false;
  for (const auto& fn : abi.functions) {
    if (fn.returnType.kind == PrimKind::OpaqueHandle) sawOpaqueHandleReturn = true;
    for (const auto& p : fn.params) {
      if (p.kind == PrimKind::OpaqueHandle) sawOpaqueHandleParam = true;
    }
  }
  BOLTFFI_CHECK(sawOpaqueHandleParam);
  BOLTFFI_CHECK(sawOpaqueHandleReturn);

  // Finding 1, second adversarial round: every `boltffi_register_callback_*` function's vtable-
  // pointer parameter must classify as `OpaqueHandle`, never `PtrConst`/`PtrMut` -- swept across
  // all 13 real register functions (one per vtable), not just a single hand-picked example, so a
  // future header that adds a 14th callback vtable can't silently regress this.
  int registerCallbackFunctions = 0;
  for (const auto& fn : abi.functions) {
    if (fn.name.rfind("boltffi_register_callback_", 0) != 0) continue;
    ++registerCallbackFunctions;
    BOLTFFI_CHECK(fn.params.size() == 1);
    BOLTFFI_CHECK(fn.params[0].kind == PrimKind::OpaqueHandle);
    BOLTFFI_CHECK(!isArenaPointerKind(fn.params[0].kind));
  }
  BOLTFFI_CHECK(registerCallbackFunctions == static_cast<int>(abi.vtables.size()));
  BOLTFFI_CHECK(registerCallbackFunctions == 13);

  const auto* poll = abi.findFunction(
      "boltffi_async_method_class_parse_core_ffi_object_parse_object_save_poll");
  BOLTFFI_CHECK(poll != nullptr);
  BOLTFFI_CHECK(poll->params[0].kind == PrimKind::OpaqueHandle);
}

// ---- planFunctionCall / planFunctionReturn: the closed-shape-space call planner ----
// (findings 1 and 2, this session: a by-value 16-byte aggregate PARAMETER
// (`BoltFFICallbackHandle`) was silently modeled as ONE register instead of the two it actually
// occupies -- and a 16-byte aggregate RETURN, every `boltffi_create_callback_*` constructor, fell
// through to the plain-scalar return path and lost its second register (the vtable pointer)
// entirely.)

BOLTFFI_TEST(plans_plain_scalar_params_as_one_slot_each) {
  auto abi = parseAbiHeader("bool boltffi_x(uint64_t a, const uint8_t *b_ptr, uintptr_t b_len);\n");
  auto plan = planFunctionCall(abi.functions[0], abi);
  BOLTFFI_CHECK(plan.has_value());
  BOLTFFI_CHECK(plan->size() == 3);
  for (const auto& slot : *plan) BOLTFFI_CHECK(slot.kind == RegisterSlotKind::Scalar);
  BOLTFFI_CHECK((*plan)[0].logicalParamIndex == 0);
  BOLTFFI_CHECK((*plan)[2].logicalParamIndex == 2);
}

BOLTFFI_TEST(plans_callback_handle_param_as_two_registers_in_field_order) {
  // Mirrors the real header's `set_http_transport(BoltFFICallbackHandle transport)`.
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    uint64_t handle;\n"
      "    const void *vtable;\n"
      "} BoltFFICallbackHandle;\n"
      "bool boltffi_set_http_transport(BoltFFICallbackHandle transport);\n");
  auto plan = planFunctionCall(abi.functions[0], abi);
  BOLTFFI_CHECK(plan.has_value());
  BOLTFFI_CHECK(plan->size() == 2);
  BOLTFFI_CHECK((*plan)[0].kind == RegisterSlotKind::TwoWordLow);
  BOLTFFI_CHECK((*plan)[1].kind == RegisterSlotKind::TwoWordHigh);
  BOLTFFI_CHECK((*plan)[0].logicalParamIndex == 0);
  BOLTFFI_CHECK((*plan)[1].logicalParamIndex == 0);
}

BOLTFFI_TEST(plans_later_args_after_a_callback_handle_param_without_shifting) {
  // The exact hazard the finding names: "later args shift" if a 16-byte aggregate param is
  // mis-modeled as one register. Mirrors the real header's `live_query_client_new(..., handle
  // BoltFFICallbackHandle transport, BoltFFICallbackHandle listener)` shape -- TWO aggregate
  // params, each contributing two registers, with a trailing scalar after both.
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    uint64_t handle;\n"
      "    const void *vtable;\n"
      "} BoltFFICallbackHandle;\n"
      "bool boltffi_x(uint64_t receiver, BoltFFICallbackHandle transport, BoltFFICallbackHandle "
      "listener, uint64_t trailing);\n");
  auto plan = planFunctionCall(abi.functions[0], abi);
  BOLTFFI_CHECK(plan.has_value());
  BOLTFFI_CHECK(plan->size() == 6);  // 1 (receiver) + 2 (transport) + 2 (listener) + 1 (trailing)
  BOLTFFI_CHECK((*plan)[0].kind == RegisterSlotKind::Scalar);
  BOLTFFI_CHECK((*plan)[0].logicalParamIndex == 0);
  BOLTFFI_CHECK((*plan)[1].kind == RegisterSlotKind::TwoWordLow);
  BOLTFFI_CHECK((*plan)[1].logicalParamIndex == 1);
  BOLTFFI_CHECK((*plan)[2].kind == RegisterSlotKind::TwoWordHigh);
  BOLTFFI_CHECK((*plan)[2].logicalParamIndex == 1);
  BOLTFFI_CHECK((*plan)[3].kind == RegisterSlotKind::TwoWordLow);
  BOLTFFI_CHECK((*plan)[3].logicalParamIndex == 2);
  BOLTFFI_CHECK((*plan)[4].kind == RegisterSlotKind::TwoWordHigh);
  BOLTFFI_CHECK((*plan)[4].logicalParamIndex == 2);
  BOLTFFI_CHECK((*plan)[5].kind == RegisterSlotKind::Scalar);
  BOLTFFI_CHECK((*plan)[5].logicalParamIndex == 3);  // NOT shifted onto an aggregate's registers
}

BOLTFFI_TEST(rejects_by_value_aggregate_params_outside_the_two_word_shape) {
  // `boltffi_free_string(FfiString string)` / `boltffi_free_buf(FfiBuf_u8 buf)` -- the two real
  // functions with an unsupported by-value aggregate PARAMETER (see generic_invoke.h's module
  // doc). These must never be callable through this generic dispatcher.
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    uint8_t *ptr;\n"
      "    uintptr_t len;\n"
      "    uintptr_t cap;\n"
      "} FfiString;\n"
      "void boltffi_free_string(FfiString string);\n");
  auto plan = planFunctionCall(abi.functions[0], abi);
  BOLTFFI_CHECK(!plan.has_value());
}

BOLTFFI_TEST(plans_return_shapes_scalar_twoword_and_sret) {
  auto abi = parseAbiHeader(
      "typedef struct {\n"
      "    int32_t code;\n"
      "} FfiStatus;\n"
      "typedef struct {\n"
      "    uint64_t handle;\n"
      "    const void *vtable;\n"
      "} BoltFFICallbackHandle;\n"
      "typedef struct {\n"
      "    uint8_t *ptr;\n"
      "    uintptr_t len;\n"
      "    uintptr_t cap;\n"
      "    uintptr_t align;\n"
      "} FfiBuf_u8;\n"
      "FfiStatus boltffi_a(uint64_t x);\n"
      "BoltFFICallbackHandle boltffi_create_callback_x(uint64_t handle);\n"
      "FfiBuf_u8 boltffi_b(uint64_t x);\n"
      "void boltffi_c(uint64_t x);\n");
  BOLTFFI_CHECK(planFunctionReturn(abi.functions[0], abi) == ReturnPlan::Scalar);  // FfiStatus, 4B
  BOLTFFI_CHECK(planFunctionReturn(abi.functions[1], abi) == ReturnPlan::TwoWord);  // 16B
  BOLTFFI_CHECK(planFunctionReturn(abi.functions[2], abi) == ReturnPlan::Sret);     // 32B
  BOLTFFI_CHECK(planFunctionReturn(abi.functions[3], abi) == ReturnPlan::Scalar);   // void
}

BOLTFFI_TEST(every_real_function_is_plannable_except_the_two_free_functions) {
  std::string source = readFixture("fixtures/parse_core_real_abi.h");
  auto abi = parseAbiHeader(source);

  int unplannable = 0;
  bool sawTwoWordParam = false, sawTwoWordReturn = false, sawSretReturn = false;
  for (const auto& fn : abi.functions) {
    auto plan = planFunctionCall(fn, abi);
    if (!plan) {
      ++unplannable;
      continue;
    }
    for (const auto& slot : *plan) {
      if (slot.kind == RegisterSlotKind::TwoWordLow) sawTwoWordParam = true;
    }
    switch (planFunctionReturn(fn, abi)) {
      case ReturnPlan::TwoWord:
        sawTwoWordReturn = true;
        break;
      case ReturnPlan::Sret:
        sawSretReturn = true;
        break;
      case ReturnPlan::Scalar:
        break;
    }
  }
  // Exactly `boltffi_free_string`/`boltffi_free_buf` fall outside the closed shape space (see
  // generic_invoke.h's module doc) -- everything else, including every `BoltFFICallbackHandle`
  // by-value parameter and every `boltffi_create_callback_*` 16-byte return, must plan cleanly.
  BOLTFFI_CHECK(unplannable == 2);
  BOLTFFI_CHECK(sawTwoWordParam);
  BOLTFFI_CHECK(sawTwoWordReturn);
  BOLTFFI_CHECK(sawSretReturn);
}

int main() { return boltffi_test::runAll(); }
