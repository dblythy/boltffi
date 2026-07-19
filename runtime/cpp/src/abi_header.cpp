#include "boltffi/abi_header.h"

#include <cctype>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <unordered_map>

namespace boltffi {

namespace {

std::string trim(std::string_view s) {
  std::size_t start = 0;
  std::size_t end = s.size();
  while (start < end && std::isspace(static_cast<unsigned char>(s[start]))) ++start;
  while (end > start && std::isspace(static_cast<unsigned char>(s[end - 1]))) --end;
  return std::string(s.substr(start, end - start));
}

bool endsWith(std::string_view s, std::string_view suffix) {
  return s.size() >= suffix.size() && s.compare(s.size() - suffix.size(), suffix.size(), suffix) == 0;
}

bool startsWith(std::string_view s, std::string_view prefix) {
  return s.size() >= prefix.size() && s.compare(0, prefix.size(), prefix) == 0;
}

/// Splits a C parameter/argument list on top-level commas only -- a comma nested inside an inline
/// function-pointer parameter's own `(...)` (e.g. `fetch`'s `void (*)(void *, FfiStatus, FfiBuf_u8)`
/// parameter) must not split there.
std::vector<std::string> splitTopLevelCommas(const std::string& text) {
  std::vector<std::string> parts;
  int depth = 0;
  std::string current;
  for (char ch : text) {
    if (ch == '(') ++depth;
    if (ch == ')') --depth;
    if (ch == ',' && depth == 0) {
      parts.push_back(trim(current));
      current.clear();
    } else {
      current.push_back(ch);
    }
  }
  parts.push_back(trim(current));
  return parts;
}

/// An inline function-pointer parameter/field-type looks like `RET (*NAME)(ARGS)` or
/// `RET (*)(ARGS)` -- detected structurally by the presence of `(*` before the type's own
/// terminating `;`/`,` (never appears in this header for any other reason: a bare pointer
/// parameter is always spelled `TYPE *name`, never `(*name)`).
bool looksLikeFnPtrType(const std::string& text) {
  return text.find("(*") != std::string::npos;
}

struct AliasTable {
  std::unordered_map<std::string, PrimKind> aliases;
  std::unordered_map<std::string, std::size_t> recordSizes;

  AliasTable() {
    aliases["void"] = PrimKind::Void;
    aliases["bool"] = PrimKind::Bool;
    aliases["int8_t"] = PrimKind::I32;   // widened; only ever used for the poll-result signal byte
    aliases["uint8_t"] = PrimKind::U32;  // only appears here as a pointee (`uint8_t *`), never bare
    aliases["int32_t"] = PrimKind::I32;
    aliases["uint32_t"] = PrimKind::U32;
    aliases["int64_t"] = PrimKind::I64;
    aliases["uint64_t"] = PrimKind::U64;
    aliases["uintptr_t"] = PrimKind::U64;
    aliases["size_t"] = PrimKind::U64;
    aliases["double"] = PrimKind::F64;
  }
};

/// Resolves a bare (non-pointer, non-function-pointer) type identifier through the alias table --
/// a primitive, an enum-underlying-type alias (`___NetworkState` -> I32/U32), or a named aggregate
/// record (`FfiBuf_u8`, `FfiStatus`, `BoltFFICallbackHandle`).
TypeRef resolveBareIdentifier(const std::string& ident, const AliasTable& aliases) {
  auto it = aliases.aliases.find(ident);
  if (it != aliases.aliases.end()) {
    TypeRef ref;
    ref.kind = it->second;
    return ref;
  }
  auto rec = aliases.recordSizes.find(ident);
  if (rec != aliases.recordSizes.end()) {
    TypeRef ref;
    ref.kind = PrimKind::Aggregate;
    ref.aggregateName = ident;
    return ref;
  }
  throw std::runtime_error("boltffi::parseAbiHeader: unknown type identifier '" + ident + "'");
}

/// Splits `"[const] IDENT [*] [name]"` into (isConst, identifier, hasPointer, trailingName).
/// `hasName` distinguishes a function *parameter* (`const uint8_t *key_ptr`) from a bare type
/// occurring where no name is expected (return types, vtable field param types).
struct TypeText {
  bool isConst = false;
  std::string identifier;
  bool isPointer = false;
  std::string name;
};

TypeText splitTypeText(const std::string& text) {
  std::string s = text;
  TypeText out;
  // Trailing name: the last token, if it's a plain identifier and there's more than one token
  // once pointer stars are accounted for. We scan tokens split on whitespace, treating a leading
  // '*' run attached to the next token (`* name` or `*name`) uniformly.
  std::string working = trim(s);
  if (startsWith(working, "const ")) {
    out.isConst = true;
    working = trim(working.substr(6));
  }
  // Count and strip all '*' (only ever 0 or 1 in this header).
  std::size_t stars = 0;
  std::string noStars;
  noStars.reserve(working.size());
  for (char ch : working) {
    if (ch == '*') {
      ++stars;
    } else {
      noStars.push_back(ch);
    }
  }
  out.isPointer = stars > 0;
  // noStars is now e.g. "uint8_t key_ptr" or "uint8_t " (no name) or "uintptr_t len" etc.,
  // possibly with extra internal whitespace where the '*' used to be.
  std::istringstream iss(noStars);
  std::vector<std::string> tokens;
  std::string tok;
  while (iss >> tok) tokens.push_back(tok);
  if (tokens.empty()) {
    throw std::runtime_error("boltffi::parseAbiHeader: empty type in '" + text + "'");
  }
  if (tokens.size() == 1) {
    out.identifier = tokens[0];
  } else {
    // identifier is everything except the last token (the parameter name); this header never
    // emits multi-word type identifiers beyond `const`, already stripped above.
    out.identifier = tokens.front();
    out.name = tokens.back();
  }
  return out;
}

/// Counts the top-level comma-separated parts of an inline function-pointer type's OWN parameter
/// list, e.g. `void (*)(void *, FfiStatus, FfiBuf_u8)` -> 3 -- used only to disambiguate the real
/// header's completion-callback shapes (see `TypeRef::fnPtrArity`'s doc); never recurses into those
/// parameters' own types.
int countFnPtrArity(const std::string& fnPtrText) {
  std::size_t starParenPos = fnPtrText.find("(*");
  std::size_t nameClose = fnPtrText.find(')', starParenPos);
  std::size_t argsOpen = fnPtrText.find('(', nameClose);
  int depth = 0;
  std::size_t open = std::string::npos, close = std::string::npos;
  for (std::size_t i = argsOpen; i < fnPtrText.size(); ++i) {
    if (fnPtrText[i] == '(') {
      if (depth == 0) open = i;
      ++depth;
    } else if (fnPtrText[i] == ')') {
      --depth;
      if (depth == 0) {
        close = i;
        break;
      }
    }
  }
  if (open == std::string::npos || close == std::string::npos) return 0;
  std::string inner = trim(fnPtrText.substr(open + 1, close - open - 1));
  if (inner.empty() || inner == "void") return 0;
  return static_cast<int>(splitTopLevelCommas(inner).size());
}

TypeRef resolveParamOrReturnType(const std::string& text, const AliasTable& aliases) {
  std::string trimmed = trim(text);
  if (looksLikeFnPtrType(trimmed)) {
    TypeRef ref;
    ref.kind = PrimKind::FnPtr;
    ref.fnPtrArity = countFnPtrArity(trimmed);
    return ref;
  }
  TypeText parts = splitTypeText(trimmed);
  if (parts.isPointer) {
    TypeRef ref;
    ref.kind = parts.isConst ? PrimKind::PtrConst : PrimKind::PtrMut;
    ref.name = parts.name;
    return ref;
  }
  TypeRef ref = resolveBareIdentifier(parts.identifier, aliases);
  ref.name = parts.name;
  return ref;
}

std::vector<TypeRef> parseParamList(const std::string& argsText, const AliasTable& aliases) {
  std::vector<TypeRef> params;
  std::string trimmed = trim(argsText);
  if (trimmed.empty() || trimmed == "void") return params;
  for (const std::string& part : splitTopLevelCommas(trimmed)) {
    params.push_back(resolveParamOrReturnType(part, aliases));
  }
  return params;
}

std::size_t primByteSize(PrimKind kind) {
  switch (kind) {
    case PrimKind::I32:
    case PrimKind::U32:
    case PrimKind::Bool:
      return 4;
    default:
      return 8;  // I64/U64/PtrConst/PtrMut/FnPtr all register/pointer-sized on a 64-bit target
  }
}

/// Finds the top-level `(` .. `)` pair delimiting a function's parameter list, given `text`
/// contains exactly one such pair at depth 0 (true of every declaration this parser accepts:
/// nested parens only ever occur INSIDE that one parameter list, e.g. an inline function-pointer
/// parameter's own arg list).
std::pair<std::size_t, std::size_t> findTopLevelParens(const std::string& text) {
  int depth = 0;
  std::size_t openPos = std::string::npos;
  for (std::size_t i = 0; i < text.size(); ++i) {
    if (text[i] == '(') {
      if (depth == 0) openPos = i;
      ++depth;
    } else if (text[i] == ')') {
      --depth;
      if (depth == 0) return {openPos, i};
    }
  }
  throw std::runtime_error("boltffi::parseAbiHeader: unbalanced parens in '" + text + "'");
}

/// Parses one plain function declaration line, e.g.
/// `FfiBuf_u8 boltffi_method_class_..._acl_set_read_access(const uint8_t *receiver_ptr, ...);`
FunctionAbi parseFunctionDecl(const std::string& line, const AliasTable& aliases) {
  std::string decl = line;
  if (!endsWith(decl, ";")) throw std::runtime_error("boltffi::parseAbiHeader: expected ';' in '" + line + "'");
  decl.pop_back();

  auto [openPos, closePos] = findTopLevelParens(decl);
  std::string beforeParen = trim(decl.substr(0, openPos));
  std::string argsText = decl.substr(openPos + 1, closePos - openPos - 1);

  // beforeParen is "RETTYPE NAME" -- NAME is the last identifier token.
  std::size_t lastSpace = beforeParen.find_last_of(" \t*");
  if (lastSpace == std::string::npos) {
    throw std::runtime_error("boltffi::parseAbiHeader: cannot split return type/name in '" + line + "'");
  }
  std::string retText = trim(beforeParen.substr(0, lastSpace + 1));
  std::string name = trim(beforeParen.substr(lastSpace + 1));

  FunctionAbi fn;
  fn.name = name;
  fn.returnType = resolveParamOrReturnType(retText, aliases);
  fn.params = parseParamList(argsText, aliases);
  return fn;
}

/// Parses one vtable field declaration, e.g. `void (*fetch)(uint64_t, ..., void (*)(...), void *);`
/// -- unlike a parameter-position function pointer, the field's OWN return/params are captured in
/// full (this is the signature the C++ trampoline must implement).
VTableFieldAbi parseVTableField(const std::string& line, const AliasTable& aliases) {
  std::string decl = line;
  if (!endsWith(decl, ";")) throw std::runtime_error("boltffi::parseAbiHeader: expected ';' in '" + line + "'");
  decl.pop_back();

  std::size_t starParenPos = decl.find("(*");
  if (starParenPos == std::string::npos) {
    throw std::runtime_error("boltffi::parseAbiHeader: expected '(*' in vtable field '" + line + "'");
  }
  std::string retText = trim(decl.substr(0, starParenPos));
  std::size_t nameClose = decl.find(')', starParenPos);
  std::string name = trim(decl.substr(starParenPos + 2, nameClose - (starParenPos + 2)));

  std::size_t argsOpen = decl.find('(', nameClose);
  auto [innerOpen, innerClose] = [&]() {
    int depth = 0;
    std::size_t open = std::string::npos;
    for (std::size_t i = argsOpen; i < decl.size(); ++i) {
      if (decl[i] == '(') {
        if (depth == 0) open = i;
        ++depth;
      } else if (decl[i] == ')') {
        --depth;
        if (depth == 0) return std::pair<std::size_t, std::size_t>(open, i);
      }
    }
    throw std::runtime_error("boltffi::parseAbiHeader: unbalanced parens in vtable field '" + line + "'");
  }();
  std::string argsText = decl.substr(innerOpen + 1, innerClose - innerOpen - 1);

  VTableFieldAbi field;
  field.name = name;
  field.returnType = resolveParamOrReturnType(retText, aliases);
  field.params = parseParamList(argsText, aliases);
  return field;
}

bool isBlankOrBoilerplate(const std::string& line) {
  if (line.empty()) return true;
  if (startsWith(line, "#")) return true;
  if (line == "extern \"C\" {" || line == "}" || line == "#endif") return true;
  return false;
}

}  // namespace

const FunctionAbi* ParsedAbi::findFunction(std::string_view name) const {
  for (const auto& fn : functions) {
    if (fn.name == name) return &fn;
  }
  return nullptr;
}

const VTableAbi* ParsedAbi::findVTable(std::string_view name) const {
  for (const auto& vt : vtables) {
    if (vt.name == name) return &vt;
  }
  return nullptr;
}

const RecordAbi* ParsedAbi::findRecord(std::string_view name) const {
  for (const auto& rec : records) {
    if (rec.name == name) return &rec;
  }
  return nullptr;
}

ParsedAbi parseAbiHeader(std::string_view source) {
  AliasTable aliases;
  ParsedAbi result;

  std::vector<std::string> lines;
  {
    std::string current;
    for (char ch : source) {
      if (ch == '\n') {
        lines.push_back(current);
        current.clear();
      } else {
        current.push_back(ch);
      }
    }
    if (!current.empty()) lines.push_back(current);
  }

  for (std::size_t i = 0; i < lines.size(); ++i) {
    std::string line = trim(lines[i]);
    if (isBlankOrBoilerplate(line)) continue;

    if (startsWith(line, "static inline")) {
      // Skip through the matching closing brace; these are the atomic-op helpers with real
      // bodies, never called generically (the adapter never needs process-local atomics).
      if (!endsWith(line, "}")) {
        while (i + 1 < lines.size() && trim(lines[i]) != "}") {
          ++i;
        }
      }
      continue;
    }

    if (startsWith(line, "#define")) continue;

    if (startsWith(line, "typedef struct")) {
      std::string structOpen = line;
      // Collect field lines until the closing `} Name;`.
      std::vector<std::string> fieldLines;
      std::string closeLine;
      while (++i < lines.size()) {
        std::string fieldLine = trim(lines[i]);
        if (startsWith(fieldLine, "}")) {
          closeLine = fieldLine;
          break;
        }
        if (!fieldLine.empty()) fieldLines.push_back(fieldLine);
      }
      if (closeLine.empty()) {
        throw std::runtime_error("boltffi::parseAbiHeader: unterminated struct starting at '" + structOpen + "'");
      }
      // closeLine looks like "} ___SessionStorageVTable;"
      std::string name = trim(closeLine.substr(1));
      if (!name.empty() && name.back() == ';') name.pop_back();
      name = trim(name);

      bool allFnPtr = !fieldLines.empty();
      for (const auto& f : fieldLines) {
        if (f == "uint8_t _unused;") continue;
        if (!looksLikeFnPtrType(f)) {
          allFnPtr = false;
          break;
        }
      }

      if (allFnPtr) {
        VTableAbi vtable;
        vtable.name = name;
        for (const auto& f : fieldLines) {
          if (f == "uint8_t _unused;") continue;
          vtable.fields.push_back(parseVTableField(f, aliases));
        }
        result.vtables.push_back(std::move(vtable));
      } else {
        std::size_t size = 0;
        for (const auto& f : fieldLines) {
          if (f == "uint8_t _unused;") {
            size += 1;
            continue;
          }
          std::string fieldDecl = f;
          if (endsWith(fieldDecl, ";")) fieldDecl.pop_back();
          std::size_t lastSpace = fieldDecl.find_last_of(" \t*");
          std::string typeText = lastSpace == std::string::npos ? fieldDecl : fieldDecl.substr(0, lastSpace + 1);
          TypeRef fieldType = resolveParamOrReturnType(typeText, aliases);
          size += fieldType.kind == PrimKind::PtrConst || fieldType.kind == PrimKind::PtrMut
                      ? 8
                      : primByteSize(fieldType.kind);
        }
        aliases.recordSizes[name] = size;
        result.records.push_back(RecordAbi{name, size});
      }
      continue;
    }

    if (startsWith(line, "typedef ")) {
      std::string rest = trim(line.substr(8));
      if (!endsWith(rest, ";")) continue;  // defensive; every real typedef line ends in ';'
      rest.pop_back();
      rest = trim(rest);

      if (looksLikeFnPtrType(rest)) {
        // `typedef RET (*Name)(ARGS);`
        std::size_t starParenPos = rest.find("(*");
        std::size_t nameClose = rest.find(')', starParenPos);
        std::string name = trim(rest.substr(starParenPos + 2, nameClose - (starParenPos + 2)));
        aliases.aliases[name] = PrimKind::FnPtr;
        continue;
      }

      // `typedef TYPE Name;` (simple alias: enum underlying type, or `const void *Name`).
      std::size_t lastSpace = rest.find_last_of(" \t*");
      if (lastSpace == std::string::npos) continue;
      std::string typeText = rest.substr(0, lastSpace + 1);
      std::string name = trim(rest.substr(lastSpace + 1));
      TypeRef resolved = resolveParamOrReturnType(typeText, aliases);
      if (resolved.kind == PrimKind::PtrConst || resolved.kind == PrimKind::PtrMut) {
        // A NAMED typedef alias resolving to a bare pointer type (`typedef const void
        // *RustFutureHandle;`, the real header's only instance) is an OPAQUE, Rust-managed handle
        // -- never a real inline pointer a generic caller may rebase against a bound arena. Every
        // OTHER `PtrConst`/`PtrMut` classification in this parser comes from an inline `T *`
        // spelling at the actual use site (`const uint8_t *key_ptr` in a param list), which never
        // goes through this branch -- so recording `OpaqueHandle` here, instead of collapsing back
        // to the same `PtrConst`/`PtrMut` kind the inline case uses, is what lets
        // `isArenaPointerKind` tell the two apart later without guessing from a parameter's name.
        aliases.aliases[name] = PrimKind::OpaqueHandle;
      } else {
        aliases.aliases[name] = resolved.kind;
      }
      continue;
    }

    if (endsWith(line, ");") && line.find('(') != std::string::npos) {
      result.functions.push_back(parseFunctionDecl(line, aliases));
      continue;
    }

    // Anything else on a recognized-boilerplate-free line is unexpected input for this header's
    // grammar; surface it loudly rather than silently mis-parsing.
    if (!line.empty()) {
      throw std::runtime_error("boltffi::parseAbiHeader: unrecognized line: '" + line + "'");
    }
  }

  return result;
}

namespace {

/// `true` iff `param` is exactly the 16-byte `BoltFFICallbackHandle` by-value shape -- the ONLY
/// by-value aggregate PARAMETER this dispatcher's register-level plan knows how to expand. Checks
/// the actual resolved record size (not just the name) so a future header that reused the name for
/// a differently-shaped record would be rejected rather than silently mis-expanded.
bool isTwoWordCallbackHandleParam(const TypeRef& param, const ParsedAbi& abi) {
  if (param.kind != PrimKind::Aggregate) return false;
  const RecordAbi* record = abi.findRecord(param.aggregateName);
  return record != nullptr && record->byteSize == 16;
}

}  // namespace

std::optional<std::vector<RegisterSlot>> planFunctionCall(const FunctionAbi& fn, const ParsedAbi& abi) {
  std::vector<RegisterSlot> plan;
  plan.reserve(fn.params.size() + 1);
  for (std::size_t i = 0; i < fn.params.size(); ++i) {
    const TypeRef& param = fn.params[i];
    if (param.kind == PrimKind::F64) {
      plan.push_back({RegisterSlotKind::Float, i});
    } else if (param.kind == PrimKind::Aggregate) {
      if (!isTwoWordCallbackHandleParam(param, abi)) {
        // Outside the closed shape space (e.g. `boltffi_free_string`/`boltffi_free_buf`'s own
        // >16-byte by-value parameters) -- `fn` must never be routed through this dispatcher.
        return std::nullopt;
      }
      plan.push_back({RegisterSlotKind::TwoWordLow, i});
      plan.push_back({RegisterSlotKind::TwoWordHigh, i});
    } else {
      plan.push_back({RegisterSlotKind::Scalar, i});
    }
  }
  return plan;
}

ReturnPlan planFunctionReturn(const FunctionAbi& fn, const ParsedAbi& abi) {
  if (fn.returnType.kind != PrimKind::Aggregate) return ReturnPlan::Scalar;
  const RecordAbi* record = abi.findRecord(fn.returnType.aggregateName);
  std::size_t byteSize = record != nullptr ? record->byteSize : 0;
  if (byteSize <= 8) return ReturnPlan::Scalar;
  if (byteSize == 16) return ReturnPlan::TwoWord;
  return ReturnPlan::Sret;
}

}  // namespace boltffi
