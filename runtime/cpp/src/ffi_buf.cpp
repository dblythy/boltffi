#include "boltffi/ffi_buf.h"

namespace boltffi {

std::vector<std::uint8_t> decodeAndFreeFfiBuf(FfiBuf buf) {
  std::vector<std::uint8_t> out;
  if (buf.ptr != nullptr && buf.len > 0) {
    out.assign(buf.ptr, buf.ptr + buf.len);
  }
  boltffi_free_buf(buf);
  return out;
}

}  // namespace boltffi
