#include "boltffi/generic_callback.h"

#include <utility>

namespace boltffi {

namespace {

// One table-builder per shape (a template template parameter can't bind to a function template in
// C++17, so this is spelled out per shape rather than shared via one generic helper -- still just
// SIX small functions, one per shape, not one per symbol/trait/method).

template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildScalarReturnTable(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineScalarReturn<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildVoidArgs0Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineVoidArgs0<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildVoidScalar1Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineVoidScalar1<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildVoidScalar2Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineVoidScalar2<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildVoidBuf1Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineVoidBuf1<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildCompletionStatus0Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineCompletionStatus0<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildCompletionStatus1Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineCompletionStatus1<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildCompletionStatus2Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineCompletionStatus2<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildCompletionStatusBuf0Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineCompletionStatusBuf0<Ids>)...}};
}
template <std::size_t... Ids>
constexpr std::array<void*, sizeof...(Ids)> buildCompletionStatusBuf1Table(std::index_sequence<Ids...>) {
  return {{reinterpret_cast<void*>(&detail::trampolineCompletionStatusBuf1<Ids>)...}};
}

}  // namespace

std::optional<CallbackShape> classifyVTableField(const VTableFieldAbi& field) {
  const auto& p = field.params;

  if (field.name == "free" && p.size() == 1 && p[0].kind == PrimKind::U64 &&
      field.returnType.kind == PrimKind::Void) {
    return CallbackShape::Free;
  }
  if (field.name == "clone" && p.size() == 1 && p[0].kind == PrimKind::U64 &&
      field.returnType.kind == PrimKind::U64) {
    return CallbackShape::Clone;
  }
  // ScalarReturn: `T(uint64_t)`, T != void -- e.g. `factor`/`now_ms` (same shape as `clone` but a
  // different field name; `clone` itself is handled above by name so it's never double-classified).
  if (p.size() == 1 && p[0].kind == PrimKind::U64 && field.returnType.kind != PrimKind::Void) {
    return CallbackShape::ScalarReturn;
  }
  // VoidArgs0: `void(uint64_t)` under any OTHER name than `free` -- `open_socket`/`close_socket`.
  if (p.size() == 1 && p[0].kind == PrimKind::U64 && field.returnType.kind == PrimKind::Void) {
    return CallbackShape::VoidArgs0;
  }
  // VoidScalar1/2: `void(uint64_t, scalar...)` -- `on_change` (1 scalar), `schedule` (2 scalars).
  if (p.size() == 2 && p[0].kind == PrimKind::U64 &&
      (p[1].kind == PrimKind::I32 || p[1].kind == PrimKind::U32 || p[1].kind == PrimKind::I64 ||
       p[1].kind == PrimKind::U64 || p[1].kind == PrimKind::Bool) &&
      field.returnType.kind == PrimKind::Void) {
    return CallbackShape::VoidScalar1;
  }
  if (p.size() == 3 && p[0].kind == PrimKind::U64 &&
      (p[1].kind == PrimKind::I32 || p[1].kind == PrimKind::U32 || p[1].kind == PrimKind::I64 ||
       p[1].kind == PrimKind::U64) &&
      (p[2].kind == PrimKind::I32 || p[2].kind == PrimKind::U32 || p[2].kind == PrimKind::I64 ||
       p[2].kind == PrimKind::U64) &&
      field.returnType.kind == PrimKind::Void) {
    return CallbackShape::VoidScalar2;
  }
  // VoidBuf1: `void(uint64_t, const uint8_t*, uintptr_t)` -- `send`/`on_event`/`on_diff`.
  if (p.size() == 3 && p[0].kind == PrimKind::U64 && p[1].kind == PrimKind::PtrConst &&
      p[2].kind == PrimKind::U64 && field.returnType.kind == PrimKind::Void) {
    return CallbackShape::VoidBuf1;
  }
  // CompletionStatus0: `void(uint64_t, void(*)(void*,int32_t), void*)` -- `clear`.
  if (p.size() == 3 && p[0].kind == PrimKind::U64 && p[1].kind == PrimKind::FnPtr &&
      p[1].fnPtrArity == 2 && p[2].kind == PrimKind::PtrMut && field.returnType.kind == PrimKind::Void) {
    return CallbackShape::CompletionStatus0;
  }
  // CompletionStatusBuf0: `void(uint64_t, void(*)(void*,int32_t,FfiBuf_u8), void*)` -- `get` (no
  // input, e.g. SessionStorage).
  if (p.size() == 3 && p[0].kind == PrimKind::U64 && p[1].kind == PrimKind::FnPtr &&
      p[1].fnPtrArity == 3 && p[2].kind == PrimKind::PtrMut && field.returnType.kind == PrimKind::Void) {
    return CallbackShape::CompletionStatusBuf0;
  }
  // CompletionStatus1: `void(uint64_t, const uint8_t*, uintptr_t, void(*)(void*,int32_t), void*)`
  // -- `set`/`delete` (1 buffer in, status-only completion).
  if (p.size() == 5 && p[0].kind == PrimKind::U64 && p[1].kind == PrimKind::PtrConst &&
      p[2].kind == PrimKind::U64 && p[3].kind == PrimKind::FnPtr && p[3].fnPtrArity == 2 &&
      p[4].kind == PrimKind::PtrMut && field.returnType.kind == PrimKind::Void) {
    return CallbackShape::CompletionStatus1;
  }
  // CompletionStatusBuf1: same shape, but the completion carries a buffer result too -- `get`/
  // `keys` (KeyValueStorage), `fetch` (HttpTransport).
  if (p.size() == 5 && p[0].kind == PrimKind::U64 && p[1].kind == PrimKind::PtrConst &&
      p[2].kind == PrimKind::U64 && p[3].kind == PrimKind::FnPtr && p[3].fnPtrArity == 3 &&
      p[4].kind == PrimKind::PtrMut && field.returnType.kind == PrimKind::Void) {
    return CallbackShape::CompletionStatusBuf1;
  }
  // CompletionStatus2: `void(uint64_t, (const uint8_t*, uintptr_t)*2, void(*)(void*,int32_t), void*)`
  // -- `KeyValueStorage::set` (key + value, status-only completion).
  if (p.size() == 7 && p[0].kind == PrimKind::U64 && p[1].kind == PrimKind::PtrConst &&
      p[2].kind == PrimKind::U64 && p[3].kind == PrimKind::PtrConst && p[4].kind == PrimKind::U64 &&
      p[5].kind == PrimKind::FnPtr && p[5].fnPtrArity == 2 && p[6].kind == PrimKind::PtrMut &&
      field.returnType.kind == PrimKind::Void) {
    return CallbackShape::CompletionStatus2;
  }
  return std::nullopt;
}

std::vector<void*> buildVTableBytes(const VTableAbi& vtable) {
  std::vector<void*> slots;
  slots.reserve(vtable.fields.size());
  for (const auto& field : vtable.fields) {
    auto shape = classifyVTableField(field);
    if (!shape) {
      slots.push_back(nullptr);
      continue;
    }
    slots.push_back(buildCallbackTrampoline(*shape, field.name));
  }
  return slots;
}

void* buildCallbackTrampoline(CallbackShape shape, const std::string& methodName) {
  if (shape == CallbackShape::Free) return reinterpret_cast<void*>(&detail::genericFree);
  if (shape == CallbackShape::Clone) return reinterpret_cast<void*>(&detail::genericClone);

  std::size_t slot = detail::allocateSlot(methodName);
  static const auto scalarReturnTable = buildScalarReturnTable(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto voidArgs0Table = buildVoidArgs0Table(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto voidScalar1Table = buildVoidScalar1Table(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto voidScalar2Table = buildVoidScalar2Table(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto voidBuf1Table = buildVoidBuf1Table(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto completionStatus0Table =
      buildCompletionStatus0Table(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto completionStatus1Table =
      buildCompletionStatus1Table(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto completionStatus2Table =
      buildCompletionStatus2Table(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto completionStatusBuf0Table =
      buildCompletionStatusBuf0Table(std::make_index_sequence<kMaxCallbackSlots>{});
  static const auto completionStatusBuf1Table =
      buildCompletionStatusBuf1Table(std::make_index_sequence<kMaxCallbackSlots>{});
  switch (shape) {
    case CallbackShape::ScalarReturn:
      return scalarReturnTable[slot];
    case CallbackShape::VoidArgs0:
      return voidArgs0Table[slot];
    case CallbackShape::VoidScalar1:
      return voidScalar1Table[slot];
    case CallbackShape::VoidScalar2:
      return voidScalar2Table[slot];
    case CallbackShape::VoidBuf1:
      return voidBuf1Table[slot];
    case CallbackShape::CompletionStatus0:
      return completionStatus0Table[slot];
    case CallbackShape::CompletionStatus1:
      return completionStatus1Table[slot];
    case CallbackShape::CompletionStatus2:
      return completionStatus2Table[slot];
    case CallbackShape::CompletionStatusBuf0:
      return completionStatusBuf0Table[slot];
    case CallbackShape::CompletionStatusBuf1:
      return completionStatusBuf1Table[slot];
    default:
      throw std::runtime_error("boltffi::generic_callback: unhandled shape");
  }
}

}  // namespace boltffi
