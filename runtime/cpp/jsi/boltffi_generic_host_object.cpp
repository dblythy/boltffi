#include "boltffi_generic_host_object.h"

#include <dlfcn.h>

#include <cstring>
#include <stdexcept>

using facebook::jsi::ArrayBuffer;
using facebook::jsi::Function;
using facebook::jsi::PropNameID;
using facebook::jsi::Runtime;
using facebook::jsi::Value;

namespace boltffi {

namespace {

/// A `jsi::MutableBuffer` wrapping an owned `std::vector<uint8_t>` -- the eager-copy convention
/// this whole adapter uses for byte-buffer-valued returns (matches `ffi_buf.h`'s own doc: no-copy
/// wrapping is a later, specifically-reviewed optimization).
class OwnedBuffer : public facebook::jsi::MutableBuffer {
 public:
  explicit OwnedBuffer(std::vector<std::uint8_t> data) : data_(std::move(data)) {}
  std::uint8_t* data() override { return data_.data(); }
  std::size_t size() const override { return data_.size(); }

 private:
  std::vector<std::uint8_t> data_;
};

}  // namespace

BoltFFIGenericHostObject::BoltFFIGenericHostObject(void* dylibHandle, ParsedAbi abi)
    : dylib_(dylibHandle), abi_(std::move(abi)) {}

void* BoltFFIGenericHostObject::resolveSymbol(const std::string& name) {
  std::lock_guard<std::mutex> lock(symbolsMutex_);
  auto it = resolvedSymbols_.find(name);
  if (it != resolvedSymbols_.end()) return it->second;
  void* sym = dlsym(dylib_, name.c_str());
  if (!sym) throw std::runtime_error("boltffi: symbol not found: " + name);
  resolvedSymbols_[name] = sym;
  return sym;
}

CValue BoltFFIGenericHostObject::translateIfPointer(const TypeRef& paramType, CValue value,
                                                     std::uint8_t* base) const {
  // `isArenaPointerKind` (abi_header.h) is `false` for `OpaqueHandle` (e.g. `RustFutureHandle`, or
  // a `boltffi_register_callback_*` vtable pointer -- finding 1, second adversarial round): an
  // opaque, process-owned handle must cross verbatim, never rebased against the arena the way a
  // REAL inline `PtrConst`/`PtrMut` data pointer (`key_ptr`, `value_ptr`, ...) is (finding 3,
  // first session).
  if (!isArenaPointerKind(paramType.kind)) return value;
  if (value.u64 == 0) return value;  // null stays null -- never offset against the arena base
  if (!base) {
    throw std::runtime_error(
        "boltffi: pointer-shaped argument received with no arena bound (call "
        "__boltffi_native_bind_arena first)");
  }
  return CValue::ofPtr(base + value.u64);
}

std::uint64_t BoltFFIGenericHostObject::callScalar(const std::string& name,
                                                    const std::vector<CValue>& registerArgs) {
  if (!abi_.findFunction(name)) throw std::runtime_error("boltffi: unknown function: " + name);
  return invokeGenericScalar(resolveSymbol(name), registerArgs.data(), registerArgs.size());
}

void BoltFFIGenericHostObject::callSretIntoBuffer(const std::string& name, const std::vector<CValue>& registerArgs,
                                                   void* outBuffer, std::size_t outSize) {
  if (!abi_.findFunction(name)) throw std::runtime_error("boltffi: unknown function: " + name);
  invokeGenericSret(resolveSymbol(name), registerArgs.data(), registerArgs.size(), outBuffer, outSize);
}

TwoWord BoltFFIGenericHostObject::callTwoWord(const std::string& name, const std::vector<CValue>& registerArgs) {
  if (!abi_.findFunction(name)) throw std::runtime_error("boltffi: unknown function: " + name);
  return invokeGenericTwoWord(resolveSymbol(name), registerArgs.data(), registerArgs.size());
}

std::vector<PropNameID> BoltFFIGenericHostObject::getPropertyNames(Runtime& rt) {
  std::vector<PropNameID> names;
  names.reserve(abi_.functions.size() + 1);
  names.push_back(PropNameID::forAscii(rt, "__boltffi_native_bind_arena"));
  for (const auto& fn : abi_.functions) {
    names.push_back(PropNameID::forUtf8(rt, fn.name));
  }
  return names;
}

Value BoltFFIGenericHostObject::get(Runtime& rt, const PropNameID& name) {
  std::string propName = name.utf8(rt);

  if (propName == "__boltffi_native_bind_arena") {
    return Function::createFromHostFunction(
        rt, name, 1,
        [this](Runtime& rt2, const Value&, const Value* args, std::size_t count) -> Value {
          if (count < 1 || !args[0].isObject() || !args[0].asObject(rt2).isArrayBuffer(rt2)) {
            throw facebook::jsi::JSError(rt2, "__boltffi_native_bind_arena expects an ArrayBuffer");
          }
          ArrayBuffer buffer = args[0].asObject(rt2).getArrayBuffer(rt2);
          // Keeps the REAL jsi::ArrayBuffer object alive so a call in flight can pin its own
          // `shared_ptr` to the CURRENT buffer before this one replaces the member -- see the
          // module doc's finding-8 note. A call derives its base address directly from its OWN
          // pin (`pinnedArena->data(rt)`, see `get()` below) rather than from any address stored
          // here, which is what eliminates the torn-snapshot hazard the second adversarial
          // round's finding 1 residual named: there is only ever one fact (this buffer object),
          // never a separate address that could disagree with it. `jsi::Pointer` is move-only, so
          // the buffer is moved into a fresh heap allocation rather than copied.
          auto held = std::make_shared<ArrayBuffer>(std::move(buffer));
          std::lock_guard<std::mutex> lock(arenaObjectMutex_);
          currentArenaBuffer_ = std::move(held);
          return Value::undefined();
        });
  }

  const FunctionAbi* fn = abi_.findFunction(propName);
  if (!fn) return Value::undefined();

  // The closed-shape-space call/return planner (findings 1+2): `nullopt` means `fn` has a
  // parameter outside the shapes this dispatcher covers (e.g. `boltffi_free_string`/
  // `boltffi_free_buf`'s own >16-byte by-value parameters) -- such a function is REJECTED here,
  // exactly like an unknown name, rather than silently mis-called through the wrong registers.
  auto plan = planFunctionCall(*fn, abi_);
  if (!plan) return Value::undefined();
  ReturnPlan returnPlan = planFunctionReturn(*fn, abi_);
  std::size_t aggregateSize = 0;
  if (returnPlan != ReturnPlan::Scalar) {
    const RecordAbi* record = abi_.findRecord(fn->returnType.aggregateName);
    aggregateSize = record ? record->byteSize : 0;
  }

  return Function::createFromHostFunction(
      rt, name, static_cast<unsigned int>(fn->params.size()),
      [this, propName, fn, plan, returnPlan, aggregateSize](Runtime& rt2, const Value&, const Value* jsArgs,
                                                             std::size_t count) -> Value {
        // Pin the CURRENTLY bound arena for this call's ENTIRE duration (finding 8): a synchronous
        // host-callback shape invoked by the native call below could reenter JS, which could grow
        // the arena (replacing `currentArenaBuffer_`) before this call returns -- a local copy of
        // the `shared_ptr` keeps the underlying buffer's backing store alive via JSI's own
        // refcounting regardless of what the MEMBER field is reassigned to mid-call. The base
        // address used to translate every pointer argument THIS call makes is derived from this
        // SAME pinned object, once, right here -- never from a separately-tracked address that a
        // concurrent rebind could tear apart from the pin (the second adversarial round's finding
        // 1 residual). Pinning the object's lifetime does not, by itself, make a reentrant grow's
        // effect on an OUT-param write coherent with what JS reads after the call returns -- that
        // half of the fix is the JS-side arena's call-in-flight growth guard (`native_arena.ts`).
        std::shared_ptr<ArrayBuffer> pinnedArena;
        {
          std::lock_guard<std::mutex> lock(arenaObjectMutex_);
          pinnedArena = currentArenaBuffer_;
        }
        std::uint8_t* pinnedBase = pinnedArena ? pinnedArena->data(rt2) : nullptr;

        std::vector<LogicalArg> logicalArgs(fn->params.size());
        for (std::size_t i = 0; i < fn->params.size() && i < count; ++i) {
          const TypeRef& paramType = fn->params[i];
          if (paramType.kind == PrimKind::F64) {
            logicalArgs[i].f64 = jsArgs[i].asNumber();
          } else if (paramType.kind == PrimKind::Aggregate) {
            // The one 16-byte `BoltFFICallbackHandle` by-value parameter shape: crosses as a
            // 16-byte ArrayBuffer, the SAME convention a 16-byte aggregate RETURN uses (see this
            // file's module doc) -- low 8 bytes = `handle`, high 8 bytes = `vtable` (a real
            // pointer, never arena-translated).
            if (!jsArgs[i].isObject() || !jsArgs[i].asObject(rt2).isArrayBuffer(rt2)) {
              throw facebook::jsi::JSError(rt2,
                                            "boltffi: " + propName + " expects a 16-byte ArrayBuffer for its "
                                            "callback-handle argument");
            }
            ArrayBuffer buf = jsArgs[i].asObject(rt2).getArrayBuffer(rt2);
            if (buf.size(rt2) < 16) {
              throw facebook::jsi::JSError(rt2, "boltffi: callback-handle ArrayBuffer must be >= 16 bytes");
            }
            std::uint8_t* bytes = buf.data(rt2);
            std::uint64_t low = 0, high = 0;
            std::memcpy(&low, bytes, sizeof(low));
            std::memcpy(&high, bytes + sizeof(low), sizeof(high));
            logicalArgs[i].u64 = low;
            logicalArgs[i].high = high;
          } else if (jsArgs[i].isBigInt()) {
            logicalArgs[i].u64 = jsArgs[i].asBigInt(rt2).asUint64(rt2);
          } else {
            // Every I32/U32/Bool/PtrConst/PtrMut/OpaqueHandle-classified argument arrives as a
            // plain JS number -- a literal scalar for the former, an ARENA OFFSET (never a real
            // address) for `PtrConst`/`PtrMut`, and a verbatim opaque token for `OpaqueHandle`
            // (see this file's module doc on `__boltffi_native_bind_arena` and finding 3).
            logicalArgs[i].u64 = static_cast<std::uint64_t>(static_cast<std::int64_t>(jsArgs[i].asNumber()));
          }
        }

        auto translate = [this, pinnedBase](const TypeRef& paramType, CValue v) {
          return translateIfPointer(paramType, v, pinnedBase);
        };
        std::vector<CValue> registerArgs = buildRegisterArgs(*fn, *plan, logicalArgs, translate);

        if (returnPlan == ReturnPlan::Sret) {
          std::vector<std::uint8_t> bytes(aggregateSize);
          callSretIntoBuffer(propName, registerArgs, bytes.data(), bytes.size());
          auto buffer = std::make_shared<OwnedBuffer>(std::move(bytes));
          return Value(rt2, ArrayBuffer(rt2, buffer));
        }
        if (returnPlan == ReturnPlan::TwoWord) {
          // Finding 2's fix: a 16-byte aggregate return (every `boltffi_create_callback_*`) used
          // to fall through to the plain-scalar path below and lose the `vtable` word entirely.
          TwoWord result = callTwoWord(propName, registerArgs);
          std::vector<std::uint8_t> bytes(16);
          std::memcpy(bytes.data(), &result.a, sizeof(result.a));
          std::memcpy(bytes.data() + sizeof(result.a), &result.b, sizeof(result.b));
          auto buffer = std::make_shared<OwnedBuffer>(std::move(bytes));
          return Value(rt2, ArrayBuffer(rt2, buffer));
        }

        std::uint64_t result = callScalar(propName, registerArgs);
        switch (fn->returnType.kind) {
          case PrimKind::Void:
            return Value::undefined();
          case PrimKind::Bool:
            return Value(result != 0);
          case PrimKind::I32:
            return Value(static_cast<double>(static_cast<std::int32_t>(result)));
          case PrimKind::U32:
            return Value(static_cast<double>(static_cast<std::uint32_t>(result)));
          default:
            // I64/U64/PtrConst/PtrMut/OpaqueHandle/small (<=8 byte) Aggregate -- BigInt is always
            // safe here (never truncates), even where a real device build might later choose a
            // plain `number` for handles the way native.ts currently does (see this session's
            // report on that discrepancy).
            return Value(rt2, facebook::jsi::BigInt::fromUint64(rt2, result));
        }
      });
}

}  // namespace boltffi
