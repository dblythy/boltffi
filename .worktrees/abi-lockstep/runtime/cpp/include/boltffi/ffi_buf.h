// Decodes the C ABI's byte-buffer return shape (`FfiBuf`/`FfiBuf_u8`,
// `boltffi_backend/templates/bridge/c/header.h:23-28`) into an owned `std::vector<uint8_t>` --
// the "solve the struct/buffer-valued return" item the react-native track's design doc assigns to
// this layer (docs/tracks/react-native.md: "decode FfiBuf on the C++ side, hand JS a plain
// ArrayBuffer (eager-copy first cut)"). Deliberately free of any `jsi::` type: the eager copy
// happens here in plain C++; only the one-line wrap into a `jsi::ArrayBuffer` (a `MutableBuffer`
// whose `data()`/`size()` return this vector's storage) needs `jsi/jsi.h`, and lives in the JSI
// adapter layer, not here.
//
// Eager copy-then-free, not a no-copy `jsi::MutableBuffer` wrapping the raw `FfiBuf` allocation
// directly: the design doc's own risk table calls out that a no-copy wrapper's destructor must
// retain the buffer's `cap` AND `align` fields (not just `ptr`), because `boltffi_free_buf`
// reconstructs a `Layout` from them to `dealloc` -- passing a reconstructed or mismatched
// `cap`/`align` is undefined behavior, and a `jsi::MutableBuffer` subclass only exposes
// `data()`/`size()` to callers, making that retention easy to drop by accident in a first
// implementation. Eager copy sidesteps the whole hazard class for this first cut (matches wasm's
// own `takeBuf*`/`takeSlot*` copy-then-free semantics, `module.ts:541-545`); a no-copy variant is
// a later, specifically-reviewed optimization.
#pragma once

#include <cstddef>
#include <cstdint>
#include <vector>

namespace boltffi {

/// Mirrors the generated `FfiBuf_u8` C struct layout EXACTLY (field order and width matter: this
/// crosses the FFI boundary by value). `usize` -> `std::size_t` (not `uintptr_t`): both are the
/// platform pointer-width unsigned integer on every target this adapter ships for (LP64 Apple/
/// Android ABIs), and matching Rust's `boltffi_core::types::buf::FfiBuf` (`ptr: *mut u8, len:
/// usize, cap: usize, align: usize`) field-for-field is what makes this layout ABI-compatible,
/// not the specific C name chosen for the integer type.
struct FfiBuf {
  std::uint8_t* ptr;
  std::size_t len;
  std::size_t cap;
  std::size_t align;
};

/// `extern "C" fn boltffi_free_buf(buf: FfiBuf)` (`boltffi_core/src/types/buf.rs:103`) -- takes
/// the struct BY VALUE and reconstructs a `Layout` from `cap`/`align` to `dealloc`. Exported by
/// every crate this adapter links (one instance per linked cdylib/xcframework/.so, standard C
/// linkage -- not redefined here).
extern "C" void boltffi_free_buf(FfiBuf buf);

/// Copies `buf`'s bytes into an owned, GC-independent `std::vector<uint8_t>`, then frees the
/// original allocation via `boltffi_free_buf`. Safe to call with an empty/null buffer (`ptr ==
/// nullptr` or `len == 0`) -- `boltffi_free_buf` itself is a no-op deallocation-wise for a
/// zero-capacity `FfiBuf` (mirrors `FfiBuf::empty()`, `buf.rs:12-19`).
std::vector<std::uint8_t> decodeAndFreeFfiBuf(FfiBuf buf);

}  // namespace boltffi
