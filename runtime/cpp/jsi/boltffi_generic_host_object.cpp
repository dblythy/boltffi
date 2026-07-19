#include "boltffi_generic_host_object.h"

#include <dlfcn.h>

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

void BoltFFIGenericHostObject::bindArena(std::uint8_t* baseAddress) { arenaBase_.store(baseAddress); }

CValue BoltFFIGenericHostObject::translateIfPointer(const TypeRef& paramType, CValue value) const {
  if (paramType.kind != PrimKind::PtrConst && paramType.kind != PrimKind::PtrMut) return value;
  if (value.u64 == 0) return value;  // null stays null -- never offset against the arena base
  std::uint8_t* base = arenaBase_.load();
  if (!base) {
    throw std::runtime_error(
        "boltffi: pointer-shaped argument received with no arena bound (call "
        "__boltffi_native_bind_arena first)");
  }
  return CValue::ofPtr(base + value.u64);
}

std::uint64_t BoltFFIGenericHostObject::callScalar(const std::string& name, const std::vector<CValue>& args) {
  const FunctionAbi* fn = abi_.findFunction(name);
  if (!fn) throw std::runtime_error("boltffi: unknown function: " + name);
  std::vector<CValue> translated;
  translated.reserve(args.size());
  for (std::size_t i = 0; i < args.size(); ++i) {
    translated.push_back(i < fn->params.size() ? translateIfPointer(fn->params[i], args[i]) : args[i]);
  }
  return invokeGenericScalar(resolveSymbol(name), translated.data(), translated.size());
}

void BoltFFIGenericHostObject::callSretIntoBuffer(const std::string& name, const std::vector<CValue>& args,
                                                   void* outBuffer, std::size_t outSize) {
  const FunctionAbi* fn = abi_.findFunction(name);
  if (!fn) throw std::runtime_error("boltffi: unknown function: " + name);
  std::vector<CValue> translated;
  translated.reserve(args.size());
  for (std::size_t i = 0; i < args.size(); ++i) {
    translated.push_back(i < fn->params.size() ? translateIfPointer(fn->params[i], args[i]) : args[i]);
  }
  invokeGenericSret(resolveSymbol(name), translated.data(), translated.size(), outBuffer, outSize);
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
          bindArena(buffer.data(rt2));
          return Value::undefined();
        });
  }

  const FunctionAbi* fn = abi_.findFunction(propName);
  if (!fn) return Value::undefined();

  // Aggregate (record) returns wider than 16 bytes need a decode buffer sized from the ABI; look
  // it up once here (not per-call) since it never changes for a given function.
  std::size_t aggregateSize = 0;
  bool isLargeAggregate = false;
  if (fn->returnType.kind == PrimKind::Aggregate) {
    const RecordAbi* record = abi_.findRecord(fn->returnType.aggregateName);
    if (record && record->byteSize > 16) {
      aggregateSize = record->byteSize;
      isLargeAggregate = true;
    }
  }

  return Function::createFromHostFunction(
      rt, name, static_cast<unsigned int>(fn->params.size()),
      [this, propName, fn, isLargeAggregate, aggregateSize](Runtime& rt2, const Value&, const Value* jsArgs,
                                                              std::size_t count) -> Value {
        std::vector<CValue> args;
        args.reserve(count);
        for (std::size_t i = 0; i < count; ++i) {
          const TypeRef& paramType = i < fn->params.size() ? fn->params[i] : TypeRef{};
          if (paramType.kind == PrimKind::F64) {
            args.push_back(CValue::ofF64(jsArgs[i].asNumber()));
          } else if (jsArgs[i].isBigInt()) {
            args.push_back(CValue::ofU64(jsArgs[i].asBigInt(rt2).asUint64(rt2)));
          } else {
            // Every I32/U32/Bool/PtrConst/PtrMut-classified argument arrives as a plain JS number
            // -- a literal scalar for the former, an ARENA OFFSET (never a real address) for the
            // latter (see this file's module doc on `__boltffi_native_bind_arena`).
            args.push_back(CValue::ofU64(static_cast<std::uint64_t>(static_cast<std::int64_t>(jsArgs[i].asNumber()))));
          }
        }

        if (isLargeAggregate) {
          std::vector<std::uint8_t> bytes(aggregateSize);
          callSretIntoBuffer(propName, args, bytes.data(), bytes.size());
          auto buffer = std::make_shared<OwnedBuffer>(std::move(bytes));
          return Value(rt2, ArrayBuffer(rt2, buffer));
        }

        std::uint64_t result = callScalar(propName, args);
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
            // I64/U64/PtrConst/PtrMut (a handle)/small (<=8 byte) Aggregate -- BigInt is always
            // safe here (never truncates), even where a real device build might later choose a
            // plain `number` for handles the way native.ts currently does (see this session's
            // report on that discrepancy).
            return Value(rt2, facebook::jsi::BigInt::fromUint64(rt2, result));
        }
      });
}

}  // namespace boltffi
