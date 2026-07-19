// The name-driven dispatcher's call mechanism (docs/tracks/react-native.md's item A: "a generic
// name-driven dispatcher ... calling them with runtime-constructed argument lists"). Deliberately
// free of any `jsi::` type -- pure C++, unit-testable directly against REAL `boltffi_*` symbols
// linked from a real `parse-core-rs` build (see tests/generic_invoke_test.cpp), same JSI-
// independent-core discipline as native_trampoline.h/ffi_buf.h.
//
// Why this needs no libffi, measured (not assumed) against the real header this fork already
// ships for every native target (tests/fixtures/parse_core_real_abi.h, 689 functions):
//   - x86-64 SysV and AArch64 AAPCS64 (the two ABIs this adapter ever targets -- iOS/macOS/Android,
//     never Windows) both pass every INTEGER-or-pointer-class argument (bool/i32/u32/i64/u64/any
//     pointer) in a general-purpose register regardless of its logical width -- a caller may treat
//     every such argument uniformly as a 64-bit value and the callee reads only the bits its own
//     declared type needs. This is the same trick that lets a single `uint64_t`-typed cast of a
//     function pointer correctly call a real `bool(uint64_t, uint64_t)`-shaped function: verified
//     against the real static lib in generic_invoke_test.cpp, not asserted from the spec alone.
//   - `double` is the ONLY other scalar class present anywhere in the real header (no `float`, no
//     other float-family type) and it uses the SEPARATE float/SSE register file on both ABIs --
//     mixing it in requires the argument's STATIC C++ type to be `double` at the exact right
//     position in the casted function-pointer type (its position relative to other args doesn't
//     matter to either register file -- only its own count/position does, and here there is always
//     at most one). Measured, not assumed: every one of the 689 real functions has AT MOST one
//     `double` parameter (enforced by `abi_header_test.cpp`'s own scan) -- so "arity N, with an
//     optional single float at position P" is a genuinely closed, small shape space (55 shapes for
//     N in 0..9), not an open-ended combinatorial one. `ShapeTable` below generates exactly this
//     space at compile time via `std::index_sequence`, once per return class.
//   - By-value struct returns split into exactly three cases, all present in the real header and
//     all handled without new dependencies: (1) a record <= 8 bytes composed of one integer field
//     (`FfiStatus`, 4 bytes) returns in a plain integer register, same as any scalar --
//     `ReturnClass::Scalar` covers it (the caller reads however many low bytes it needs). (2)
//     exactly 16 bytes, two integer-class eightbytes (`BoltFFICallbackHandle`) returns in TWO
//     registers -- modeled by a genuinely POD `TwoWord` C++ return type, which the compiler
//     classifies identically to the real struct because two adjacent `uint64_t`s classify the same
//     way under both ABIs' rules -- `ReturnClass::TwoWord`. (3) anything larger (`FfiBuf_u8` 32
//     bytes, `FfiString` 24 bytes) is returned via a hidden output pointer the CALLER allocates
//     ("sret") -- modeled by casting to a REAL large-struct C++ return type (`LargeAggregate`) and
//     letting the compiler generate whichever ABI-correct sret convention the platform actually
//     uses -- `ReturnClass::Sret`. IMPORTANT, found and fixed empirically this session: a hand-
//     rolled "cast the function as void and pass the sret pointer as a normal leading argument"
//     version of this is WRONG on AArch64 (it works on x86-64 SysV, where the hidden pointer IS a
//     normal first argument, but AAPCS64 passes it in a dedicated indirect-result register, x8,
//     completely separate from the normal x0-x7 argument registers -- see `LargeAggregate`'s own
//     doc for the reproduction). No aggregate wider than 16 bytes is ever passed BY VALUE as a
//     parameter anywhere a generic caller needs to route (the two functions that do --
//     `boltffi_free_string`/`boltffi_free_buf` -- are called directly by `ffi_buf.h`, never through
//     this generic path).
//
// If a future crate change ever introduced more than one `double` per function, or a struct
// parameter/return this module doesn't model, `abi_header_test.cpp`'s exhaustive scan over the real
// header would catch the new shape immediately (its own assertions encode today's measured bound) --
// this header's job is only to cover the shape space THAT TEST proves is real.
#pragma once

#include <array>
#include <cstdint>
#include <cstring>
#include <stdexcept>
#include <type_traits>
#include <utility>

namespace boltffi {

constexpr std::size_t kMaxGenericArity = 12;  // measured real max is 9; kept with headroom

/// One runtime-tagged call argument. Every non-float value (scalar, bool, pointer, handle) is
/// carried as a raw 64-bit bit pattern -- see the module doc for why that's ABI-safe.
struct CValue {
  bool isFloat = false;
  union {
    std::uint64_t u64;
    double f64;
  };

  static CValue ofU64(std::uint64_t v) {
    CValue c;
    c.isFloat = false;
    c.u64 = v;
    return c;
  }
  static CValue ofPtr(const void* p) { return ofU64(reinterpret_cast<std::uint64_t>(p)); }
  static CValue ofF64(double v) {
    CValue c;
    c.isFloat = true;
    c.f64 = v;
    return c;
  }
};

/// A genuinely POD two-eightbyte aggregate -- classifies identically to `BoltFFICallbackHandle`
/// (`{ uint64_t; const void*; }`) under both AAPCS64 and x86-64 SysV, so a function pointer cast
/// with this as its return type receives the real two-register return correctly.
struct TwoWord {
  std::uint64_t a;
  std::uint64_t b;
};

/// Caller-side storage for a large (>16 byte) by-value aggregate return (`FfiBuf_u8`, 32 bytes;
/// `FfiString`, 24 bytes -- `kLargeAggregateCap` has headroom over both). CRITICAL: this must be
/// used as a REAL C++ struct return type in the casted function-pointer type, never emulated by
/// "cast the function as void and pass a hidden pointer parameter" -- that x86-64-SysV-shaped trick
/// is flat-out WRONG on AArch64 (verified empirically this session: AAPCS64 passes a large
/// aggregate's indirect-result pointer in a DEDICATED register, x8, completely separate from the
/// normal argument registers x0-x7 a hand-rolled "leading pointer argument" would occupy -- calling
/// through a `void(*)(RetPtr, args...)`-shaped cast on arm64 corrupts the call and reliably crashes
/// with SIGBUS/SIGSEGV, reproduced with both a synthetic local aggregate-returning function and a
/// real `parse-core-rs` symbol before this fix). Declaring the REAL aggregate type as the return of
/// the casted function pointer lets the C++ compiler emit whichever convention the target ABI
/// actually requires (x8 indirect-result on AAPCS64, a normal hidden first argument on x86-64
/// SysV) -- this is the one return shape in this file that must go through the compiler's own
/// aggregate-return code generation rather than a hand-rolled register-passing trick.
constexpr std::size_t kLargeAggregateCap = 64;
struct LargeAggregate {
  unsigned char bytes[kLargeAggregateCap];
};

namespace detail {

template <typename T>
inline T extractArg(const CValue& v);
template <>
inline double extractArg<double>(const CValue& v) {
  return v.f64;
}
template <>
inline std::uint64_t extractArg<std::uint64_t>(const CValue& v) {
  return v.u64;
}

template <std::size_t I, int FloatPos>
using ArgTypeAt = std::conditional_t<(static_cast<int>(I) == FloatPos), double, std::uint64_t>;

/// Casts `fn` to the exact `Ret(ArgTypeAt<0>, ArgTypeAt<1>, ...)` shape for a fixed arity (given by
/// `Is...`) and single optional float position `FloatPos`, and calls it with `args`.
template <typename Ret, int FloatPos, std::size_t... Is>
Ret invokeIndexed(void* fn, const CValue* args, std::index_sequence<Is...>) {
  using FnPtr = Ret (*)(ArgTypeAt<Is, FloatPos>...);
  return reinterpret_cast<FnPtr>(fn)(extractArg<ArgTypeAt<Is, FloatPos>>(args[Is])...);
}

template <typename Ret, std::size_t N, int FloatPos>
Ret invokeShape(void* fn, const CValue* args) {
  return invokeIndexed<Ret, FloatPos>(fn, args, std::make_index_sequence<N>{});
}

using GenericThunk = void (*)();  // opaque storage; cast back to the real Ret(void*,const CValue*)

template <typename Ret, std::size_t N, int... ShiftedFloatPositions>
constexpr std::array<Ret (*)(void*, const CValue*), sizeof...(ShiftedFloatPositions)> buildShapeTableImpl(
    std::integer_sequence<int, ShiftedFloatPositions...>) {
  // ShiftedFloatPositions runs 0..N (inclusive); position 0 means "no float" (FloatPos = -1),
  // position k (k>=1) means "float at index k-1".
  return {{&invokeShape<Ret, N, ShiftedFloatPositions - 1>...}};
}

/// A compile-time table of every "(arity N, optional single float at position P)" shape's invoker,
/// indexed at runtime by `floatPos + 1` (0 == no float).
template <typename Ret, std::size_t N>
struct ShapeTable {
  static const std::array<Ret (*)(void*, const CValue*), N + 1>& table() {
    static const auto instance = buildShapeTableImpl<Ret, N>(std::make_integer_sequence<int, static_cast<int>(N) + 1>{});
    return instance;
  }
};

template <typename Ret>
Ret dispatchArity(void* fn, const CValue* args, std::size_t arity, int floatPos) {
  // clang-format off
  switch (arity) {
    case 0: return ShapeTable<Ret, 0>::table()[floatPos + 1](fn, args);
    case 1: return ShapeTable<Ret, 1>::table()[floatPos + 1](fn, args);
    case 2: return ShapeTable<Ret, 2>::table()[floatPos + 1](fn, args);
    case 3: return ShapeTable<Ret, 3>::table()[floatPos + 1](fn, args);
    case 4: return ShapeTable<Ret, 4>::table()[floatPos + 1](fn, args);
    case 5: return ShapeTable<Ret, 5>::table()[floatPos + 1](fn, args);
    case 6: return ShapeTable<Ret, 6>::table()[floatPos + 1](fn, args);
    case 7: return ShapeTable<Ret, 7>::table()[floatPos + 1](fn, args);
    case 8: return ShapeTable<Ret, 8>::table()[floatPos + 1](fn, args);
    case 9: return ShapeTable<Ret, 9>::table()[floatPos + 1](fn, args);
    case 10: return ShapeTable<Ret, 10>::table()[floatPos + 1](fn, args);
    case 11: return ShapeTable<Ret, 11>::table()[floatPos + 1](fn, args);
    case 12: return ShapeTable<Ret, 12>::table()[floatPos + 1](fn, args);
    default: throw std::runtime_error("boltffi::generic_invoke: arity exceeds kMaxGenericArity");
  }
  // clang-format on
}

/// Finds the (at most one) float argument's position, or -1. Throws if more than one is found --
/// `abi_header_test.cpp` proves this never happens for the real crate, but a caller passing a
/// hand-built arg list should still fail loudly rather than silently mis-dispatch.
inline int singleFloatPosition(const CValue* args, std::size_t count) {
  int pos = -1;
  for (std::size_t i = 0; i < count; ++i) {
    if (args[i].isFloat) {
      if (pos != -1) throw std::runtime_error("boltffi::generic_invoke: more than one float argument");
      pos = static_cast<int>(i);
    }
  }
  return pos;
}

}  // namespace detail

/// Calls `fn` (resolved elsewhere, e.g. via `dlsym`) with `args`, treating it as a function
/// returning a plain scalar/pointer/handle (or a <=8-byte one-integer-field aggregate like
/// `FfiStatus`, e.g. `bool`/`int32_t`/`int64_t`/`uint64_t`/a pointer). The caller narrows the
/// returned 64-bit value to whatever width the real return type declares.
inline std::uint64_t invokeGenericScalar(void* fn, const CValue* args, std::size_t argc) {
  if (argc > kMaxGenericArity) throw std::runtime_error("boltffi::generic_invoke: too many arguments");
  int floatPos = detail::singleFloatPosition(args, argc);
  return detail::dispatchArity<std::uint64_t>(fn, args, argc, floatPos);
}

/// Calls `fn` treating it as returning `void` -- used both for genuinely void-returning functions
/// and for the sret convention (see `invokeGenericSret`, which prepends the hidden pointer and
/// calls through here).
inline void invokeGenericVoid(void* fn, const CValue* args, std::size_t argc) {
  if (argc > kMaxGenericArity) throw std::runtime_error("boltffi::generic_invoke: too many arguments");
  int floatPos = detail::singleFloatPosition(args, argc);
  detail::dispatchArity<void>(fn, args, argc, floatPos);
}

/// Calls `fn` treating it as returning a 16-byte, two-integer-eightbyte aggregate BY VALUE
/// (`BoltFFICallbackHandle`'s shape: `{ uint64_t; const void*; }`) -- the only such shape the real
/// header contains.
inline TwoWord invokeGenericTwoWord(void* fn, const CValue* args, std::size_t argc) {
  if (argc > kMaxGenericArity) throw std::runtime_error("boltffi::generic_invoke: too many arguments");
  int floatPos = detail::singleFloatPosition(args, argc);
  return detail::dispatchArity<TwoWord>(fn, args, argc, floatPos);
}

/// Calls `fn` (a function whose C return type is an aggregate wider than 16 bytes, e.g.
/// `FfiBuf_u8`/`FfiString`) and copies the first `outSize` bytes of the result into `outBuffer`
/// (caller-allocated, at least `outSize` bytes -- typically the real return type's
/// `RecordAbi::byteSize`). Dispatches through `LargeAggregate`, a real C++ struct return type, so
/// the compiler generates the ABI-correct indirect-result convention for the target platform (see
/// `LargeAggregate`'s doc for why the naive "hidden pointer parameter" version of this is wrong on
/// AArch64). `outSize` must not exceed `kLargeAggregateCap`.
inline void invokeGenericSret(void* fn, const CValue* args, std::size_t argc, void* outBuffer, std::size_t outSize) {
  if (outSize > kLargeAggregateCap) throw std::runtime_error("boltffi::generic_invoke: aggregate exceeds kLargeAggregateCap");
  if (argc > kMaxGenericArity) throw std::runtime_error("boltffi::generic_invoke: too many arguments");
  int floatPos = detail::singleFloatPosition(args, argc);
  LargeAggregate result = detail::dispatchArity<LargeAggregate>(fn, args, argc, floatPos);
  std::memcpy(outBuffer, result.bytes, outSize);
}

}  // namespace boltffi
