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

using boltffi::parseAbiHeader;
using boltffi::PrimKind;

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

BOLTFFI_TEST(pointer_typedef_alias_used_as_bare_return_type) {
  auto abi = parseAbiHeader(
      "typedef const void *RustFutureHandle;\n"
      "RustFutureHandle boltffi_x_start(uint64_t receiver);\n");
  BOLTFFI_CHECK(abi.functions[0].returnType.kind == PrimKind::PtrConst);
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
}

int main() { return boltffi_test::runAll(); }
