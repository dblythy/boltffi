// The native-mode counterpart to `callback.txt`'s wasm-only `_callbackImports` wiring
// (docs/tracks/react-native.md, parse-core-sdks repo). In wasm mode, a JS-implemented callback
// trait's methods become imports the `WebAssembly.Instance`'s import object supplies at
// instantiation time -- there is no such thing in native mode, where the compiled artifact is a
// real dylib linked/dlopen'd ahead of time. A native callback vtable is instead a real block of
// memory whose slots are genuine native function pointers, installed via the trait's
// `boltffi_register_callback_*` export (`runtime/cpp/include/boltffi/generic_callback.h`'s own
// doc describes the identical Rust-side contract this file's `bootstrapCallbackVTable` mirrors:
// the real generated register function stores the raw pointer FOREVER in a process-wide
// `static AtomicPtr` -- there is no unregister, and registering twice would leak a fresh vtable
// allocation on every callback instance construction, never just once per trait).
//
// Mirrors `native.ts`'s own `NativeTrampolineFactory<Token>` abstraction for the async
// continuation trampoline: this file owns no backend-specific mechanism for turning a JS closure
// into a real native function pointer (Bun's `JSCallback` in this repo's own tests; a JSI
// `CallInvoker`-backed C++ trampoline in a real RN host) -- that stays the HOST's job, supplied
// as a `NativeCallbackTokenFactory`. This file only owns the bookkeeping every backend shares:
// per-trait vtable memoization, and the per-instance id/ref-count registry `callback.txt` bakes
// into every generated wasm-mode trait today (`_{{trait}}_registry`/`_{{trait}}_ref_counts`),
// reused generically here instead of duplicated once per trait in generated code.

/** A native-callable function pointer wrapping a JS closure, plus its owning backend's cleanup
 * (Bun's `JSCallback.close()`; a JSI adapter's equivalent, if it has one). Opaque to this file --
 * never inspected beyond `.ptr`. */
export interface NativeCallbackToken<Ptr> {
  readonly ptr: Ptr;
  close?(): void;
}

/** The closed register-class shape space `generic_invoke.h`/`generic_callback.h` (runtime/cpp)
 * already established for the C++ adapter -- every callback vtable field this repo generates
 * reduces to these (bool/i8/u8/.../u64/any pointer are all ONE general-purpose register
 * regardless of declared width on both x86-64 SysV and AArch64 AAPCS64; `f64` is the only
 * float-class value ever seen). A host's token factory maps this closed vocabulary onto whatever
 * its own native-callback primitive needs (Bun FFI's `FFIType`, a JSI trampoline's C++ signature). */
export type NativeCallbackScalarType = "u64" | "i64" | "u32" | "i32" | "bool" | "ptr" | "f64";

export interface NativeCallbackShape {
  readonly args: readonly NativeCallbackScalarType[];
  readonly returns: NativeCallbackScalarType | "void";
  /** Whether Rust may invoke this slot from a thread other than the one that registered it (an
   * async completion's background-thread fire, mirroring `NativeTrampolineFactory`'s own
   * `onSignal` doc) -- the host's factory is responsible for the actual thread-hop (Bun's
   * `JSCallback({ threadsafe: true })`; a JSI `CallInvoker::invokeAsync`), never this file. */
  readonly threadsafe?: boolean;
}

/** Host-provided: wraps `invoke` as a genuine native function pointer callable from Rust through
 * a vtable slot, matching `shape`. */
export type NativeCallbackTokenFactory<Ptr> = (
  invoke: (...args: never[]) => unknown,
  shape: NativeCallbackShape
) => NativeCallbackToken<Ptr>;

export interface NativeCallbackHostExports {
  [key: string]: unknown;
}

/**
 * One process-wide vtable registration per callback trait -- see this module's own header doc for
 * why re-registering the SAME trait must never happen twice: the real `boltffi_register_callback_*`
 * stores the pointer it's given forever, so a second registration would silently orphan the first
 * vtable's storage (still reachable from any handle minted against it) while a NEW allocation
 * takes over for every future call, and (if the two happen to differ, e.g. across hot-reloads)
 * risks handles from before the second registration dispatching through a vtable Rust no longer
 * points at. Keyed by `registerFnName` (one per trait) rather than by content, matching
 * `registerVTableForProcessLifetime`'s (runtime/cpp) own "leak once, on purpose" discipline. Value
 * type is `unknown`, not `bigint` — see `bootstrapCallbackVTable`'s own doc on why the vtable
 * pointer's real type is host-dependent.
 */
const registeredVTables = new Map<string, unknown>();

/** Test-only: clears the process-wide registration cache (real production code never needs this --
 * a real trait is registered at most once per process, by definition). */
export function _resetCallbackVTableRegistrationsForTests(): void {
  registeredVTables.clear();
}

/**
 * Builds one trait's vtable bytes from `fieldPtrs` (free, clone, then each trait method, in the
 * exact order the real `#[repr(C)] struct ...VTable` declares them -- `boltffi_bindgen`'s callback
 * lowering always emits free/clone first) via the caller-supplied `writeVTableBytes` (however the
 * host's own allocator -- `NativeMemoryArena`, a raw Bun buffer, ... -- backs native memory; this
 * file has no allocator of its own, matching `native.ts`'s "type + delegate" discipline), then
 * calls `registerFnName` with the result exactly once for the trait's whole process lifetime.
 * Returns the SAME vtable pointer on every later call for the same `registerFnName`, never
 * re-registering or re-allocating.
 */
// `Ptr` is intentionally generic (not hardcoded to `bigint`, unlike `fieldPtrs`' element type,
// which is always raw memory CONTENT — an 8-byte slot value inside the vtable buffer, so always a
// plain integer regardless of host): a real native pointer VALUE this file hands back opaquely
// to `registerFnName`/a later `create_fn` call is whatever type the host's OWN FFI binding needs
// to accept for a pointer-typed parameter -- Bun's `bun:ffi` requires its own `Pointer` (or a raw
// typed array it auto-converts), not an arbitrary `bigint`; a JSI adapter might use a real
// pointer-sized integer instead. This file never inspects or converts the value itself.
export function bootstrapCallbackVTable<Ptr>(
  exports: NativeCallbackHostExports,
  registerFnName: string,
  fieldPtrs: readonly bigint[],
  writeVTableBytes: (fieldPtrs: readonly bigint[]) => Ptr
): Ptr {
  const cached = registeredVTables.get(registerFnName);
  if (cached !== undefined) return cached as Ptr;
  const vtablePtr = writeVTableBytes(fieldPtrs);
  const registerFn = exports[registerFnName] as ((ptr: Ptr) => void) | undefined;
  if (!registerFn) {
    throw new Error(`native callback registration export not found: ${registerFnName}`);
  }
  registerFn(vtablePtr);
  registeredVTables.set(registerFnName, vtablePtr);
  return vtablePtr;
}

/**
 * Reads `len` bytes starting at the RAW native address `ptr` -- NOT an offset into
 * `NativeMemoryArena` (see `native_arena.ts`'s own doc: every arena offset is relative to a
 * JS-owned buffer shared with the host, but a callback-vtable slot's incoming buffer parameter is
 * a genuine process address `transfer_deferred_callback_bytes`/a Rust-stack-scoped borrow handed
 * straight to the vtable's C function pointer -- unrelated to the arena's own address space).
 * Reading arbitrary native memory by real address is an inherently host-specific capability (Bun's
 * `toArrayBuffer(ptr, ...)`; a JSI adapter's own `memcpy` off a raw pointer JS passed through as a
 * `bigint`) -- this file owns no mechanism of its own, matching `NativeCallbackTokenFactory`'s own
 * "the host's job" discipline.
 */
export type NativeCallbackByteReader = (ptr: bigint, len: bigint) => Uint8Array;

/**
 * Wraps a RAW incoming native function pointer (`ptr`, matching `shape`) as a JS-callable
 * function -- the inverse of `NativeCallbackTokenFactory` (which turns a JS closure INTO a native
 * pointer). Needed for the async-completion half of the callback-vtable contract: a deferred
 * dispatch slot (`AbiCallbackMethod::is_deferred_dispatch`) receives its own completion callback
 * as a trailing `(fnPtr, userdata)` pair PASSED IN BY THE CALLER (Rust) on every invocation --
 * there is no separately named `_complete` export to call later, unlike a class's own async
 * method (`native.ts`'s `NativeAsyncFutureManager`/`completeAsync`, which poll/complete through
 * fixed, per-method exports instead). Bun's `bun:ffi` `CFunction` is the concrete mechanism a Bun
 * host supplies; a JSI adapter would resolve the raw pointer against its own generic invocation
 * helper (`runtime/cpp/include/boltffi/generic_invoke.h`).
 */
export type NativeForeignFunctionWrapper = (
  ptr: bigint,
  shape: NativeCallbackShape
) => (...args: readonly bigint[]) => bigint | void;

/**
 * Everything native-mode callback-vtable codegen (`callback.txt`'s `native_async` branch) needs
 * from the host beyond the continuation-trampoline factory `native.ts` already threads through --
 * bundled into one object so `instantiateBoltFFINative`'s signature grows by exactly one
 * (optional) parameter regardless of how many callback traits a given module declares. Every field
 * is a HOST capability (see each type's own doc) -- this module supplies only the
 * backend-agnostic bookkeeping (`bootstrapCallbackVTable`, `NativeCallbackTraitRegistry`) built on
 * top of them.
 */
export interface NativeCallbackHostAdapter<Ptr = unknown> {
  readonly createToken: NativeCallbackTokenFactory<Ptr>;
  readonly writeVTableBytes: (fieldPtrs: readonly bigint[]) => Ptr;
  readonly readForeignBytes: NativeCallbackByteReader;
  readonly wrapForeignFunction: NativeForeignFunctionWrapper;
}

/**
 * Per-trait instance registry -- mirrors `callback.txt`'s wasm-mode `_{{trait}}_registry`/
 * `_{{trait}}_ref_counts` Maps exactly (same id-allocation/ref-counting semantics: the FIRST
 * `register()` call for a given impl mints a fresh id with ref count 1; every further reference
 * to the SAME already-registered impl should go through `retain`, never a fresh `register`),
 * reused generically here instead of one hand-duplicated copy per trait in generated code.
 */
export class NativeCallbackTraitRegistry<Impl> {
  private readonly registry = new Map<bigint, Impl>();
  private readonly refCounts = new Map<bigint, number>();
  private nextId = 1n;

  register(impl: Impl): bigint {
    const id = this.nextId;
    this.nextId += 1n;
    this.registry.set(id, impl);
    this.refCounts.set(id, 1);
    return id;
  }

  retain(id: bigint): bigint {
    const count = this.refCounts.get(id);
    if (count === undefined) throw new Error(`unknown callback handle ${id}`);
    this.refCounts.set(id, count + 1);
    return id;
  }

  lookup(id: bigint): Impl {
    const impl = this.registry.get(id);
    if (impl === undefined) throw new Error(`callback handle ${id} not found`);
    return impl;
  }

  /** Returns `true` the one time the ref count reaches zero (the caller should then release any
   * host-side resources this instance's registration owns -- e.g. closing its own tokens). Every
   * OTHER call decrements and returns `false`. */
  release(id: bigint): boolean {
    const count = this.refCounts.get(id);
    if (count === undefined) throw new Error(`unknown callback handle ${id}`);
    if (count === 1) {
      this.registry.delete(id);
      this.refCounts.delete(id);
      return true;
    }
    this.refCounts.set(id, count - 1);
    return false;
  }
}
