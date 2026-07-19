// A pure-C++ (no jsi:: dependency) parser for the C header boltffi already emits for every native
// target (`boltffi_backend/templates/bridge/c/header.h`, shipped inside
// `parse-core-rs/dist/apple/*.xcframework/*/Headers` and `dist/android/*/jni/*.h`). This is the
// "generic name-driven dispatcher"'s metadata source (docs/tracks/react-native.md, parse-core-sdks
// repo, item A of the react-native track's stage-4 remainder): rather than hand-writing C++ per
// `ParseClient`/`ParseObject`/`ParseQuery` method (~690 functions in a real build), the adapter
// parses the SAME header Swift/Kotlin/C# codegen already treats as ground truth, at build time,
// into a small typed table the generic dispatcher (`generic_invoke.h`) calls through.
//
// Why parse text instead of adding a new boltffi codegen target: the header is already complete,
// already shipped, and already the thing every other consumer trusts -- this file's parser is
// scoped to exactly the grammar boltffi's C renderer actually emits (verified against the real,
// ~690-function `parse_core.h`, see tests/fixtures/parse_core_real_abi.h), not general C.
#pragma once

#include <cstddef>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace boltffi {

/// The C type of one function parameter or return value, collapsed to exactly the kinds the real
/// header uses (verified: no `float`, no by-value aggregate parameter wider than 16 bytes other
/// than the two `free_*` utility functions this parser's callers never need to route generically --
/// see generic_invoke.h's module doc for why 16-byte-or-smaller aggregates need no special
/// parameter handling at all).
enum class PrimKind {
  Void,
  Bool,
  I32,
  U32,
  I64,
  U64,
  F64,       // `double` -- passed/returned in the float/SSE register class, never the integer one
             // (verified: the real header never uses `float`, only `double`, and never mixes more
             // than one per function -- see generic_invoke.h's module doc)
  PtrConst,  // `const T *` spelled INLINE at the use site (`const uint8_t *key_ptr`) -- a real
             // address into caller-owned memory (the JS-simulated arena, for a generic host
             // object's JS-facing call) that a translator MAY legitimately rebase.
  PtrMut,    // `T *` spelled INLINE at the use site -- same rebasing eligibility as `PtrConst`,
             // just an out-param the callee writes through instead of a read-only in-param.
  FnPtr,     // a function pointer (a named typedef like `RustFutureContinuationCallback`, or an
             // inline `RET (*)(ARGS)` parameter such as a vtable's own method fields)
  Aggregate, // a struct passed/returned BY VALUE (`FfiBuf_u8`, `FfiString`, `BoltFFICallbackHandle`,
             // `FfiStatus`, ...) -- `aggregateName` names which one; see RecordAbi for its layout.
  OpaqueHandle,  // a pointer-shaped value reached through a NAMED TYPEDEF ALIAS to a bare pointer
                 // type (today: `typedef const void *RustFutureHandle;`, the header's only such
                 // alias) rather than an inline `T *` spelling -- a Rust-owned opaque token (here,
                 // a real `Box`/`Arc` pointer the async-future machinery manages), NEVER an offset
                 // into a JS-simulated arena. Distinguishing this from `PtrConst`/`PtrMut`
                 // structurally (by how the type was spelled in the header, not by guessing from a
                 // parameter's name) is what makes it impossible for a generic caller to rebase an
                 // opaque handle against the wrong base address -- see `isArenaPointerKind`.
};

/// Whether a parameter/return of this kind is a real address into caller-owned memory that a
/// generic JS-facing caller may legitimately rebase against a bound arena (`PtrConst`/`PtrMut`,
/// e.g. `key_ptr`/`value_ptr`) -- as opposed to an opaque, Rust-managed handle (`OpaqueHandle`,
/// e.g. `RustFutureHandle`) that must cross unmodified, verbatim, never offset against anything.
/// The one place this distinction must be consulted before doing pointer arithmetic on a generic
/// argument.
inline bool isArenaPointerKind(PrimKind kind) { return kind == PrimKind::PtrConst || kind == PrimKind::PtrMut; }

/// One function parameter or a vtable method field's parameter.
struct TypeRef {
  PrimKind kind = PrimKind::Void;
  std::string aggregateName;  // populated iff kind == Aggregate
  std::string name;           // the C parameter name (function params only; empty for vtable
                               // field param types, which the header never names)
  int fnPtrArity = -1;        // populated iff kind == FnPtr: the inline function pointer's OWN
                               // param count (e.g. 2 for `void (*)(void *, FfiStatus)`, 3 for
                               // `void (*)(void *, FfiStatus, FfiBuf_u8)`) -- enough to distinguish
                               // the real header's two completion-callback shapes without a full
                               // recursive parse of the nested signature's own param TYPES.

  bool isAggregate() const { return kind == PrimKind::Aggregate; }
};

/// A plain, top-level `extern "C"` function declaration -- one entry per real `boltffi_*` symbol
/// (constructors, sync/async methods, register/create_callback, record methods, free functions).
struct FunctionAbi {
  std::string name;
  TypeRef returnType;
  std::vector<TypeRef> params;
};

/// One field of a callback vtable struct, in declared order. The first two fields are always the
/// standard `free`/`clone` lifecycle pair (see the header's own convention, mirrored by
/// `boltffi_counter_host_object.h`'s `MultiplierVTable` comment) -- this parser does not special-
/// case that; it just records fields in order, and callers may assume fields[0]/[1] are free/clone
/// for any vtable with 2+ fields (true of every vtable this fork's C renderer emits).
struct VTableFieldAbi {
  std::string name;
  TypeRef returnType;
  std::vector<TypeRef> params;
};

/// A `typedef struct { ... } ___FooVTable;` block whose fields are ALL function pointers -- the
/// header's convention for a host-callback trait's vtable (`___SessionStorageVTable`,
/// `___HttpTransportVTable`, ...). Plain data records (`FfiBuf_u8`, `FfiStatus`, ...) are NOT
/// vtables (see RecordAbi) even though they're also `typedef struct { ... } Name;` blocks --
/// distinguished structurally: a vtable's fields are 100% function-pointer-typed.
struct VTableAbi {
  std::string name;
  std::vector<VTableFieldAbi> fields;
};

/// A plain data record passed/returned BY VALUE (`FfiBuf_u8`, `FfiString`, `BoltFFICallbackHandle`,
/// `FfiStatus`). `byteSize` is derived structurally from the field list (every field in this
/// header's records is either a 4-byte `int32_t`/`uint32_t` or an 8-byte pointer/`uintptr_t`/
/// `uint64_t` -- see generic_invoke.h for how `byteSize` drives the sret-vs-register-return
/// choice: >16 bytes needs a hidden out-pointer, <=16 bytes returns in one or two registers).
struct RecordAbi {
  std::string name;
  std::size_t byteSize = 0;
};

/// Everything `parseAbiHeader` recovered from one header source blob.
struct ParsedAbi {
  std::vector<FunctionAbi> functions;
  std::vector<VTableAbi> vtables;
  std::vector<RecordAbi> records;

  const FunctionAbi* findFunction(std::string_view name) const;
  const VTableAbi* findVTable(std::string_view name) const;
  const RecordAbi* findRecord(std::string_view name) const;
};

/// Parses `source` (the full text of a boltffi-generated C header) into a `ParsedAbi`. Tolerant of
/// constructs the real header contains but this adapter never needs to call generically
/// (`#define` enum constants, `static inline` atomic helpers with real bodies, `#pragma`/
/// `#include`/`extern "C"` boilerplate) -- these are skipped, not errors. Malformed input a real
/// boltffi build would never produce (e.g. an unterminated struct) throws `std::runtime_error`.
ParsedAbi parseAbiHeader(std::string_view source);

// ---------------------------------------------------------------------------------------------
// The closed-shape-space call planner (the fix for the CRITICAL finding that a by-value 16-byte
// aggregate PARAMETER, `BoltFFICallbackHandle`, was silently modeled as a single 64-bit register
// slot instead of the two it actually occupies -- e.g. `set_http_transport(BoltFFICallbackHandle)`
// or `live_query_client_new(..., BoltFFICallbackHandle transport, BoltFFICallbackHandle
// listener)`, where every register after the FIRST such parameter was shifted by one). Rather than
// leave that gap implicit, this makes the shape space CLOSED BY CONSTRUCTION: every parameter/
// return classifies into exactly one of a small number of known-safe register shapes, or the
// function is rejected -- there is no fourth "assume it's a plain scalar" bucket a new ABI shape
// could silently fall into.
// ---------------------------------------------------------------------------------------------

/// One physical calling-convention register slot a generic caller's `CValue` array must supply for
/// ONE logical (ABI-declared) parameter. A `Scalar`/`Float` parameter needs exactly one slot;
/// `BoltFFICallbackHandle` -- the ONLY by-value aggregate parameter shape the real header uses,
/// verified against every function in `parse_core_real_abi.h` -- needs exactly two, in FIELD ORDER
/// (`handle` first, `vtable` second), because both x86-64 SysV and AAPCS64 classify a 16-byte,
/// two-INTEGER-eightbyte struct passed by value identically to two consecutive plain integer/
/// pointer arguments (see generic_invoke.h's module doc for the underlying register-class
/// reasoning this rests on).
enum class RegisterSlotKind {
  Scalar,      // bool/i32/u32/i64/u64/PtrConst/PtrMut/PtrConst-arena/OpaqueHandle/FnPtr
  Float,       // the (at most one) `double` argument -- untouched by this expansion, kept for
               // completeness so a caller can build the full register-level plan from ONE table.
  TwoWordLow,  // BoltFFICallbackHandle's `handle: uint64_t` field
  TwoWordHigh, // BoltFFICallbackHandle's `vtable: const void *` field -- NEVER arena-translated;
               // it is always a real process address returned by `boltffi_create_callback_*`.
};

/// One entry in a function's register-level calling plan: which slot kind to emit, and which
/// LOGICAL parameter (index into `FunctionAbi::params`) it was expanded from -- a caller uses this
/// to know which JS-facing/logical value to pull the slot's bits from.
struct RegisterSlot {
  RegisterSlotKind kind;
  std::size_t logicalParamIndex;
};

/// Classifies `fn`'s entire parameter list into the register-level plan `RegisterSlot`s describe,
/// or returns `std::nullopt` if ANY parameter falls outside the closed shape space this dispatcher
/// covers -- today, that means any by-value `Aggregate` parameter OTHER than the 16-byte
/// `BoltFFICallbackHandle` shape (e.g. `boltffi_free_string`/`boltffi_free_buf`'s own 24/32-byte
/// by-value parameters). A `nullopt` result means `fn` must NEVER be exposed/invoked through the
/// generic (no-libffi) dispatcher -- there is no partial or best-effort fallback; the caller must
/// either reject the call loudly or resolve the symbol through a hand-written, non-generic path
/// instead (as `ffi_buf.h` already does for the two free functions).
std::optional<std::vector<RegisterSlot>> planFunctionCall(const FunctionAbi& fn, const ParsedAbi& abi);

/// How a function's RETURN value crosses back from the real ABI call -- the register-count-driven
/// classification `generic_invoke.h`'s three entry points (`invokeGenericScalar`/
/// `invokeGenericTwoWord`/`invokeGenericSret`) already implement, exposed here as ONE piece of
/// metadata so a caller (the JSI host object's `get()`) never has to re-derive "is this aggregate
/// >16 bytes" ad hoc -- the prior gap: only the `> 16` case was ever wired up, so a 16-byte
/// aggregate return (every `boltffi_create_callback_*` constructor) silently fell through to the
/// plain-scalar path and lost its second register (the vtable pointer) entirely.
enum class ReturnPlan {
  Scalar,   // void, or any register/float/<=8-byte-one-field-aggregate return (`FfiStatus`)
  TwoWord,  // exactly the 16-byte `BoltFFICallbackHandle` aggregate shape
  Sret,     // any aggregate wider than 16 bytes (`FfiBuf_u8`, `FfiString`)
};

/// Classifies `fn`'s return type. Never fails: every return shape the real header uses (scalar,
/// <=8-byte aggregate, exactly-16-byte aggregate, >16-byte aggregate) is covered -- unlike
/// parameters, there is no "unsupported by-value aggregate return" case in the real header today
/// (verified: `abi_header_test.cpp`'s full-header scan asserts every record's byte size is one of
/// 4/16/24/32).
ReturnPlan planFunctionReturn(const FunctionAbi& fn, const ParsedAbi& abi);

}  // namespace boltffi
