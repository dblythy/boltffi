// Pure C++ unit tests for boltffi/ffi_buf.h's eager-copy-then-free decode. Uses a test-local
// `boltffi_free_buf` spy (matching Rust's real `extern "C" fn boltffi_free_buf(buf: FfiBuf)`
// signature exactly) rather than linking a real compiled crate: this target's job is to prove
// the C++ decode CONTRACT (copy the right bytes, then free with the exact fields received, no
// more no less) -- the real cross-language ABI proof already lives in the fork's own Bun harness
// (runtime/typescript/test/native.bun.test.ts) against a real compiled dylib.
#include "boltffi/ffi_buf.h"

#include <cstring>

#include "test_harness.h"

namespace {
int g_free_calls = 0;
boltffi::FfiBuf g_last_freed{};
}  // namespace

// Overrides the real symbol for this test binary only (never linked against a real crate here).
extern "C" void boltffi::boltffi_free_buf(boltffi::FfiBuf buf) {
  ++g_free_calls;
  g_last_freed = buf;
  delete[] buf.ptr;
}

BOLTFFI_TEST(decode_copies_bytes_and_frees_exactly_once) {
  g_free_calls = 0;
  constexpr std::size_t kLen = 5;
  auto* storage = new std::uint8_t[kLen]{1, 2, 3, 4, 5};
  boltffi::FfiBuf buf{storage, kLen, kLen, alignof(std::uint8_t)};

  auto decoded = boltffi::decodeAndFreeFfiBuf(buf);

  BOLTFFI_CHECK(decoded.size() == kLen);
  for (std::size_t i = 0; i < kLen; ++i) {
    BOLTFFI_CHECK(decoded[i] == i + 1);
  }
  BOLTFFI_CHECK(g_free_calls == 1);
  BOLTFFI_CHECK(g_last_freed.ptr == storage);
  BOLTFFI_CHECK(g_last_freed.len == kLen);
  BOLTFFI_CHECK(g_last_freed.cap == kLen);
  BOLTFFI_CHECK(g_last_freed.align == alignof(std::uint8_t));
}

BOLTFFI_TEST(decode_handles_empty_buffer_without_touching_null_ptr) {
  g_free_calls = 0;
  boltffi::FfiBuf empty{nullptr, 0, 0, 1};

  auto decoded = boltffi::decodeAndFreeFfiBuf(empty);

  BOLTFFI_CHECK(decoded.empty());
  BOLTFFI_CHECK(g_free_calls == 1);  // still frees (a no-op on the real Rust side) -- never skips.
}

BOLTFFI_TEST(decoded_vector_owns_its_storage_independent_of_the_original_buffer) {
  constexpr std::size_t kLen = 3;
  auto* storage = new std::uint8_t[kLen]{9, 8, 7};
  boltffi::FfiBuf buf{storage, kLen, kLen, alignof(std::uint8_t)};

  auto decoded = boltffi::decodeAndFreeFfiBuf(buf);
  // `storage` has been freed by now (the spy's `delete[]`) -- `decoded` must not depend on it.
  BOLTFFI_CHECK(decoded.size() == 3);
  BOLTFFI_CHECK(decoded[0] == 9 && decoded[1] == 8 && decoded[2] == 7);
}

int main() { return boltffi_test::runAll(); }
