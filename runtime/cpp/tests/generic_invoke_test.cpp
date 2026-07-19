// Unit tests for the generic (no-libffi) call mechanism (boltffi/generic_invoke.h). Pure C++, no
// jsi:: dependency. Covers the measured real shape space (arity 0-9, at most one `double`, three
// return classes) via synthetic local functions -- see real_crate_integration_test.cpp (gated on a
// real rn_poc build) for the complementary proof against genuine parse-core-family symbols.
#include "boltffi/generic_invoke.h"

#include <cstring>

#include "test_harness.h"

using namespace boltffi;

extern "C" {

int32_t addThree(int32_t a, int32_t b, int32_t c) { return a + b + c; }

// Mirrors the real header's `within_radians_value`-style shape: a `double` NOT in the last
// position, mixed with pointer/scalar args either side of it.
int32_t mixedFloatPosition(uint64_t a, uint64_t b, double c, uint64_t d, uint64_t e) {
  return static_cast<int32_t>(a + b + d + e + static_cast<uint64_t>(c));
}

bool boolReturn(uint64_t a, uint64_t b) { return a == b; }

void voidNoArgs() {}

struct Small4 {
  int32_t code;
};
Small4 smallAggregateReturn(int32_t n) { return Small4{n}; }

struct TwoWordReal {
  uint64_t a;
  const void* b;
};
TwoWordReal twoWordAggregateReturn(uint64_t handle) { return TwoWordReal{handle, reinterpret_cast<const void*>(0xdeadbeefULL)}; }

struct Large32 {
  uint64_t a, b, c, d;
};
Large32 largeAggregateReturn(int32_t n) { return Large32{static_cast<uint64_t>(n), 99, 0, 0}; }

}  // extern "C"

BOLTFFI_TEST(scalar_dispatch_zero_arity) {
  auto r = invokeGenericScalar(reinterpret_cast<void*>(&voidNoArgs), nullptr, 0);
  (void)r;  // void return read as 0; just proving the 0-arity shape doesn't crash
}

BOLTFFI_TEST(scalar_dispatch_plain_integers) {
  CValue args[3] = {CValue::ofU64(1), CValue::ofU64(2), CValue::ofU64(3)};
  auto r = invokeGenericScalar(reinterpret_cast<void*>(&addThree), args, 3);
  BOLTFFI_CHECK(static_cast<int32_t>(r) == 6);
}

BOLTFFI_TEST(scalar_dispatch_bool_return) {
  CValue args[2] = {CValue::ofU64(42), CValue::ofU64(42)};
  auto r = invokeGenericScalar(reinterpret_cast<void*>(&boolReturn), args, 2);
  BOLTFFI_CHECK(r == 1);
  CValue args2[2] = {CValue::ofU64(1), CValue::ofU64(2)};
  auto r2 = invokeGenericScalar(reinterpret_cast<void*>(&boolReturn), args2, 2);
  BOLTFFI_CHECK(r2 == 0);
}

BOLTFFI_TEST(scalar_dispatch_float_not_in_last_position) {
  CValue args[5] = {CValue::ofU64(1), CValue::ofU64(2), CValue::ofF64(10.0), CValue::ofU64(3), CValue::ofU64(4)};
  auto r = invokeGenericScalar(reinterpret_cast<void*>(&mixedFloatPosition), args, 5);
  BOLTFFI_CHECK(static_cast<int32_t>(r) == 20);
}

BOLTFFI_TEST(scalar_dispatch_pointer_argument) {
  int value = 99;
  CValue args[2] = {CValue::ofPtr(&value), CValue::ofPtr(&value)};
  auto r = invokeGenericScalar(reinterpret_cast<void*>(&boolReturn), args, 2);
  BOLTFFI_CHECK(r == 1);  // same pointer value compared equal through the u64 bit-pattern path
}

BOLTFFI_TEST(rejects_more_than_one_float_argument) {
  CValue args[2] = {CValue::ofF64(1.0), CValue::ofF64(2.0)};
  bool threw = false;
  try {
    invokeGenericScalar(reinterpret_cast<void*>(&addThree), args, 2);
  } catch (const std::exception&) {
    threw = true;
  }
  BOLTFFI_CHECK(threw);
}

BOLTFFI_TEST(small_aggregate_return_reads_as_scalar) {
  // A <=8-byte one-integer-field aggregate (mirrors real `FfiStatus`) classifies identically to a
  // plain int32 return -- no special handling needed, just the ordinary scalar path.
  CValue args[1] = {CValue::ofU64(7)};
  auto r = invokeGenericScalar(reinterpret_cast<void*>(&smallAggregateReturn), args, 1);
  BOLTFFI_CHECK(static_cast<int32_t>(r) == 7);
}

BOLTFFI_TEST(two_word_aggregate_return) {
  CValue args[1] = {CValue::ofU64(42)};
  TwoWord tw = invokeGenericTwoWord(reinterpret_cast<void*>(&twoWordAggregateReturn), args, 1);
  BOLTFFI_CHECK(tw.a == 42);
  BOLTFFI_CHECK(tw.b == 0xdeadbeefULL);
}

BOLTFFI_TEST(large_aggregate_return_via_sret) {
  // The critical regression test for this session's empirically-found-and-fixed bug: a naive
  // "cast as void, pass a hidden pointer parameter" sret emulation crashes on AArch64 (verified
  // with both a synthetic function and a real parse-core-rs symbol before the fix) -- this proves
  // the ACTUAL fix (a real C++ aggregate return type) works.
  CValue args[1] = {CValue::ofU64(55)};
  struct { uint64_t a, b, c, d; } out{};
  invokeGenericSret(reinterpret_cast<void*>(&largeAggregateReturn), args, 1, &out, sizeof(out));
  BOLTFFI_CHECK(out.a == 55);
  BOLTFFI_CHECK(out.b == 99);
}

BOLTFFI_TEST(large_aggregate_sret_rejects_oversized_output) {
  CValue args[1] = {CValue::ofU64(1)};
  struct { uint64_t a, b, c, d, e, f, g, h, i, j; } tooBig{};  // 80 bytes > kLargeAggregateCap
  bool threw = false;
  try {
    invokeGenericSret(reinterpret_cast<void*>(&largeAggregateReturn), args, 1, &tooBig, sizeof(tooBig));
  } catch (const std::exception&) {
    threw = true;
  }
  BOLTFFI_CHECK(threw);
}

int main() { return boltffi_test::runAll(); }
