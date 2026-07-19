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
  PtrConst,  // `const T *` -- caller-owned, read-only for the duration of the call
  PtrMut,    // `T *` -- an out-param the callee writes through
  FnPtr,     // a function pointer (a named typedef like `RustFutureContinuationCallback`, or an
             // inline `RET (*)(ARGS)` parameter such as a vtable's own method fields)
  Aggregate, // a struct passed/returned BY VALUE (`FfiBuf_u8`, `FfiString`, `BoltFFICallbackHandle`,
             // `FfiStatus`, ...) -- `aggregateName` names which one; see RecordAbi for its layout.
};

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

}  // namespace boltffi
