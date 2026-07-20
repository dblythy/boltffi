// The native BoltFFIModule backend (docs/tracks/react-native.md, parse-core-sdks repo, stages
// 1-4). Where module.ts drives generated code against `WebAssembly.Instance.exports` and wasm
// linear memory, this file drives the SAME generated-code shape against a real native ABI the
// host has already dlopen'd/linked (Bun FFI in this fork's own test harness today; a JSI
// HostObject eventually) — no wasm import object, no growth invalidation to defend against on
// the native SIDE (a native pointer is just an address).
//
// Stage 4 closes the buffer-marshaling gap stages 1-2 deliberately left open: `NativeBoltFFIModule`
// now reimplements `BoltFFIModule`'s full method surface (`allocString`/`allocWriter`/
// `takePackedBuffer`/`takeBufXArray`/... — see the class's own doc below) against a pure-JS
// `NativeMemoryArena` (`native_arena.ts`) instead of `WebAssembly.Memory`. A byte-buffer-valued
// RETURN still needs a real memory accessor on the native side to decode `FfiBuf`/`#[data]`
// records/`ParseValue` — that decode step is the stage-3/4 C++ JSI adapter's job
// (`runtime/cpp/include/boltffi/ffi_buf.h`'s `decodeAndFreeFfiBuf`), which this file assumes
// copies bytes into the arena (or hands back a plain `Uint8Array`) before `_module`'s decode
// helpers ever run — this file owns the JS-side half of that contract, not the C++ half.
//
// Call-in-flight growth marks (native_arena.ts's `beginCall`/`endCall`, wired here): every native
// dispatch this file itself makes -- `NativeAsyncFutureManager.dispatchPoll` (the initial poll
// registration AND every `MaybeReady`-queued re-poll), `NativeBoltFFIModule.completeAsync`, and
// `takeLastErrorMessage`'s two calls -- brackets JUST that one synchronous native invocation with
// the arena's growth guard, closing the reentrant-growth coherence gap `native_arena.ts`'s class
// doc describes for every route that goes through one of these three funnels (which is every
// generated async completion/poll/last-error call site — see `boltffi_bindgen`'s `class.txt`/
// `async_function.txt`/`value_type_companion.txt`/`enum_namespace.txt`, all of which call through
// `_module.completeAsync`/`_module.asyncManager.pollAsyncNative`/`_module.takeLastErrorMessage`
// rather than dispatching directly). What this does NOT yet close: a plain, non-async native call
// a generated `function.txt` function makes directly against `_exports.ffiName(...)` (and the one
// synchronous "start the async op" call `async_function.txt` emits before ever touching
// `pollAsyncNative`) never funnels through `_module` at all -- there is nothing in THIS file to
// bracket, and wiring it needs a codegen change (routing those calls through a new marked
// `_module` wrapper), out of this file's scope. See `runtime/cpp/jsi/boltffi_generic_host_object.h`
// for why the C++ HostObject cannot close that remaining gap on its own either.

import { WireReader, WireWriter } from "./wire.js";
import type { WasmWireWriterAllocator } from "./wire.js";
import { BoltFFICancelledError } from "./module.js";
import type { PrimitiveBufferElementType } from "./module.js";
import { NativeMemoryArena } from "./native_arena.js";
import type { NativeCallbackHostAdapter } from "./native_callback.js";

/**
 * Matches `boltffi_core::runtime::future::RustFuturePoll` (`extern "C" fn(u64, i8)`) — the ONLY
 * two signals the native continuation callback ever carries. Unlike wasm's `poll_sync`, which
 * returns Ready/Pending/Cancelled/Panicked directly, the native protocol's registration callback
 * only ever reports Ready or MaybeReady; cancellation and panics are discovered later, when the
 * generated code calls the method's own `_complete` export and reads its `FfiStatus` out-param
 * (see `readNativeStatusCode` below) — a real, deliberate asymmetry with wasm, not a gap.
 */
export const enum NativeContinuationSignal {
  Ready = 0,
  MaybeReady = 1,
}

/** Opaque native future/class handle. Always a real process address or an opaque handle id —
 * never round-tripped through a JS `number` (see the design doc's handle-safety note); always a
 * `bigint` here so callers never accidentally lose precision truncating a 64-bit value. */
export type NativeHandle = bigint;

/**
 * Creates the ONE continuation trampoline every async method's `poll` registration reuses for
 * the lifetime of a `NativeAsyncFutureManager` — mirrors the design doc's "the C++ continuation
 * trampoline is one fixed, generic extern C function, reused for every async method," just
 * expressed as whatever native-callable value the host's own FFI binding needs (a Bun
 * `JSCallback`, eventually a C++ function pointer for the JSI adapter). `@boltffi/runtime` never
 * assumes which host produced it — `Token` is opaque to this file.
 *
 * `onSignal` MUST be safe to invoke from any thread the real Rust waker fires the callback on
 * (see `boltffi_core::runtime::continuation::ContinuationScheduler` and this repo's own PoC
 * evidence in docs/tracks/react-native.md): the host's trampoline is responsible for hopping onto
 * the JS-owning thread via its own sanctioned primitive (Bun's `JSCallback({ threadsafe: true })`
 * today; `facebook::react::CallInvoker::invokeAsync` for the eventual JSI adapter) UNCONDITIONALLY
 * — including the same-call race where the real future resolves synchronously inside the very
 * `poll()` call that registered it. `onSignal` itself performs no thread-safety work; it assumes
 * the host already did.
 */
export type NativeTrampolineFactory<Token> = (
  onSignal: (callbackData: bigint, signal: NativeContinuationSignal) => void
) => Token;

/** The registration-style poll a generated native-async method calls: NOT a synchronous status
 * query (unlike wasm's `pollSync`) — it registers `callback` against `handle`/`callbackData` and
 * returns immediately either way. The real completion (Ready) or an intermediate wake
 * (MaybeReady, requiring a repoll) arrives later through `onSignal` above, on whatever thread the
 * Rust waker fires on. */
export type NativePollFn<Token> = (
  handle: NativeHandle,
  callbackData: bigint,
  callback: Token
) => void;

interface NativePendingFuture<Token> {
  readonly handle: NativeHandle;
  readonly poll: NativePollFn<Token>;
  readonly resolve: (handle: NativeHandle) => void;
}

/**
 * The narrow `NativeMemoryArena.beginCall()`/`endCall()` surface (native_arena.ts) every native
 * dispatch this file makes must bracket, in a `try`/`finally`, around the ACTUAL native invocation
 * only — never around an `await` (see `native_arena.ts`'s "reentrant growth" class doc for the
 * coherence bug this closes: a synchronous host-callback the native call triggers can otherwise
 * grow the arena mid-call, silently orphaning an out-param write the callee already made into the
 * OLD buffer). Kept structural — rather than importing `NativeMemoryArena` by name — so
 * `NativeAsyncFutureManager` stays constructible on its own (as several tests below already do)
 * without wiring up a real arena; `NativeBoltFFIModule` passes its own arena, which already
 * implements this shape.
 */
export interface NativeCallMarks {
  beginCall(): void;
  endCall(): void;
}

const NO_OP_CALL_MARKS: NativeCallMarks = {
  beginCall(): void {},
  endCall(): void {},
};

/**
 * The native-protocol counterpart to `AsyncFutureManager` (module.ts) — a genuinely different
 * poll loop, not a smaller version of the same thing (design doc item 4). Owns exactly one
 * continuation trampoline (created once, via `createTrampoline`) and a table of pending
 * registrations keyed by a manager-assigned `callbackData` id (never the raw `handle`, since
 * multiple in-flight calls against unrelated handles must never collide on the same table key —
 * and because the id must be stable/comparable as a `bigint` before the callback ever fires).
 *
 * Deliberately resolves the returned promise with the bare `handle`, never a decoded result:
 * mirrors wasm's `AsyncFutureManager.pollAsync` exactly, so generated code completes/frees the
 * future itself afterward (`_complete_ffi_name`/`_free_ffi_name`), the same shape either backend
 * emits.
 *
 * Every actual `poll(...)` dispatch — the initial registration AND every later re-poll a
 * `MaybeReady` signal queues — goes through `dispatchPoll`, which brackets JUST that one
 * synchronous call with `callMarks.beginCall()`/`endCall()`. The `Promise` returned by
 * `pollAsyncNative` itself is NEVER bracketed: the mark must not span the await gap between one
 * poll dispatch and the next (growth there is legal — nothing is mid-call), only the synchronous
 * window each dispatch itself runs in.
 */
export class NativeAsyncFutureManager<Token = unknown> {
  private readonly trampoline: Token;
  private readonly pending = new Map<bigint, NativePendingFuture<Token>>();
  private nextCallbackData = 1n;
  private readonly callMarks: NativeCallMarks;

  constructor(createTrampoline: NativeTrampolineFactory<Token>, callMarks: NativeCallMarks = NO_OP_CALL_MARKS) {
    this.trampoline = createTrampoline((callbackData, signal) => this.onSignal(callbackData, signal));
    this.callMarks = callMarks;
  }

  pollAsyncNative(handle: NativeHandle, poll: NativePollFn<Token>): Promise<NativeHandle> {
    return new Promise((resolve) => {
      const callbackData = this.allocateCallbackData();
      this.pending.set(callbackData, { handle, poll, resolve });
      // Registration itself only touches the lock-free ContinuationScheduler (see
      // continuation.rs) — safe to call directly on the calling (JS) thread, and it may resolve
      // the Ready case synchronously inside this very call (the "Waked" race the design doc
      // calls out). `onSignal` handles that identically to an off-thread firing: the pending
      // entry is already in the map before `poll` runs.
      this.dispatchPoll(poll, handle, callbackData);
    });
  }

  /** Brackets ONE synchronous `poll(...)` dispatch with the arena's call-in-flight growth guard
   * (see the class doc) — shared by the initial registration (`pollAsyncNative`) and every
   * `MaybeReady`-queued re-poll (`onSignal`), since both are equally a native call whose own
   * synchronous host-callbacks (if any) must not be allowed to grow the arena out from under it. */
  private dispatchPoll(poll: NativePollFn<Token>, handle: NativeHandle, callbackData: bigint): void {
    this.callMarks.beginCall();
    try {
      poll(handle, callbackData, this.trampoline);
    } finally {
      this.callMarks.endCall();
    }
  }

  private allocateCallbackData(): bigint {
    const id = this.nextCallbackData;
    this.nextCallbackData += 1n;
    return id;
  }

  private onSignal(callbackData: bigint, signal: NativeContinuationSignal): void {
    const entry = this.pending.get(callbackData);
    // A signal for an id we no longer track (already resolved, or the manager was torn down)
    // is dropped, never thrown — the raw callback thread must never be the place a JS exception
    // originates from.
    if (!entry) return;

    if (signal === NativeContinuationSignal.MaybeReady) {
      // Deferred via microtask, NOT re-issued inline. `poll()` invokes its continuation callback
      // synchronously, at most once per call (boltffi_core::runtime::future::RustFuture::poll) --
      // but `ContinuationScheduler::store_continuation` (continuation.rs) delivers MaybeReady
      // synchronously, inline, whenever the scheduler is already `Waked` the instant it runs. That
      // is not just a rare cross-thread race: any future that (re)wakes itself before returning
      // `Poll::Pending` (a legitimate pattern -- e.g. one built on a cooperative-yield primitive)
      // hits it on EVERY poll. Re-issuing `poll()` inline here would then recurse one JS + one
      // native stack frame per iteration with no bound, for a future that never actually blocks.
      // A microtask hop trades stack depth for queue depth: each iteration returns to an empty
      // stack before the next one runs, so the recursion is unbounded in iteration count but
      // bounded (O(1)) in stack depth. Each re-poll is its OWN synchronous dispatch -- routed
      // through `dispatchPoll` so it gets its OWN beginCall/endCall bracket, never one held open
      // across the microtask hop itself (growth between re-polls is legal; only the call inside
      // one is not).
      queueMicrotask(() => this.dispatchPoll(entry.poll, entry.handle, callbackData));
      return;
    }

    this.pending.delete(callbackData);
    entry.resolve(entry.handle);
  }
}

/** `sizeof(FfiStatus)` (`boltffi_backend/templates/bridge/c/header.h`: `{ int32_t code; }`) — a
 * scratch out-param buffer for a native `_complete` call's status. Because native and JS share one
 * process address space, a plain JS-allocated buffer already IS a valid native memory address
 * (no wasm-style cross-allocation dance) — the host's FFI binding passes it as a raw pointer arg
 * directly (e.g. Bun's `ptr(buffer)`). */
export function allocNativeStatusBuffer(): Uint8Array {
  return new Uint8Array(4);
}

/** Reads the `FfiStatus.code` a native `_complete` export wrote into `buffer` (little-endian
 * `int32_t`, matching every other multi-byte field this wire/ABI ever encodes). */
export function readNativeStatusCode(buffer: Uint8Array): number {
  return new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength).getInt32(0, true);
}

/** `FfiStatus` codes this repo's fixtures actually observe (`header.h`'s `FFI_STATUS_*` macros).
 * Not exhaustive of every code a real crate could return — callers should treat any non-OK,
 * non-CANCELLED code as a generic failure and consult `_panic_message` (when the method exports
 * one) before assuming "internal error" over "panic". */
export const enum NativeFfiStatus {
  Ok = 0,
  InvalidArgument = 3,
  Cancelled = 4,
}

const NATIVE_FFI_BUF_DESCRIPTOR_SIZE = 16;
const NATIVE_FFI_STRING_SIZE = 12;

export interface NativeStringAlloc {
  ptr: number;
  len: number;
}

export type NativePrimitiveBufferElementType = PrimitiveBufferElementType;

export interface NativePrimitiveBufferAlloc {
  ptr: number;
  len: number;
  allocationSize: number;
}

/**
 * The native-backend `BoltFFIModule` counterpart (seam items 1/2b/3, docs/tracks/react-native.md).
 * Reimplements `BoltFFIModule`'s ENTIRE public surface — same method names, same numeric
 * ptr/len/packed-bigint/buf-descriptor conventions — against a `NativeMemoryArena` instead of
 * `WebAssembly.Memory`. This is a deliberate design choice, not the only one possible: the
 * generated TS output is backend-agnostic at the call-site level (`TsParam::wrapper_code()`/
 * `cleanup_code()` in `boltffi_bindgen` emit `_module.allocString(...)`/`_module.allocWriter(...)`/
 * `WireWriter.ptr` identically for both targets, and `callback.txt`'s host-callback response
 * encoding calls `_module.writeBufDescriptor(out_ptr, writer.ptr, ...)` where `writer.ptr`'s
 * TYPE — `number`, on the shared `WireWriter` class every codec reuses — is fixed regardless of
 * backend). Reusing wasm's exact numeric contract, backed by a pure-JS arena instead of asking
 * every call site to pass real buffer VALUES across, means every existing wrapper_code/
 * cleanup_code/callback-encoding call site needs ZERO codegen changes to work against this class —
 * only two gaps needed closing on the codegen side (the native_async param-alloc guard and the
 * native_async packed-return branch, both in `boltffi_bindgen`), not a parallel marshaling
 * convention.
 *
 * `exports` stays the RAW, unaugmented native surface (whatever named symbols the host bound —
 * real `boltffi_*` functions in production, a plain mock `Record<string, Function>` in tests) —
 * this class never merges synthesized alloc/free functions into it. The arena is 100% private:
 * every `boltffi_wasm_alloc`-shaped operation used internally here is pure JS arithmetic, never a
 * call through `exports`. The ONE exception is `takeLastErrorMessage`, which genuinely must cross
 * into the host's real `boltffi_last_error_message`/`boltffi_free_string` symbols (there is no
 * arena-only equivalent of "ask Rust for its thread-local last error") — those calls pass an
 * arena offset as the out-param pointer, exactly like wasm passes a wasm-linear-memory offset;
 * a real JSI adapter resolves that offset against the arena's shared backing buffer (see
 * `native_arena.ts`'s module doc for the sharing contract).
 */
export class NativeBoltFFIModule<Exports, Token = unknown> {
  readonly exports: Exports;
  readonly asyncManager: NativeAsyncFutureManager<Token>;
  private readonly arena: NativeMemoryArena;
  private readonly encoder = new TextEncoder();
  private readonly decoder = new TextDecoder("utf-8");
  private readonly hostCallbackAdapter?: NativeCallbackHostAdapter<Token>;

  constructor(
    exports: Exports,
    createContinuationTrampoline: NativeTrampolineFactory<Token>,
    arena: NativeMemoryArena = new NativeMemoryArena(),
    callbackHost?: NativeCallbackHostAdapter<Token>
  ) {
    this.exports = exports;
    this.arena = arena;
    this.hostCallbackAdapter = callbackHost;
    // `arena` itself satisfies `NativeCallMarks` (it has its own `beginCall`/`endCall`) -- every
    // poll dispatch the async manager makes shares THIS instance's own call-in-flight growth guard.
    this.asyncManager = new NativeAsyncFutureManager<Token>(createContinuationTrampoline, arena);
    this.bindArenaToHost();
  }

  /**
   * The host-supplied callback-vtable bridge (`native_callback.ts`'s `NativeCallbackHostAdapter`)
   * every `native_async` callback-trait's generated registration/dispatch code reads through --
   * throws loudly rather than returning `undefined` silently, matching this file's own
   * "throw rather than skip" discipline (`takeLastErrorMessage`'s missing-export branch is the ONE
   * deliberate exception, because a last-error string is optional; a callback trait a consumer
   * actually registers is not).
   */
  get callbackHost(): NativeCallbackHostAdapter<Token> {
    if (!this.hostCallbackAdapter) {
      throw new Error(
        "boltffi: no NativeCallbackHostAdapter was supplied to instantiateBoltFFINative -- a " +
          "callback trait cannot be registered without one (see native_callback.ts's own doc)."
      );
    }
    return this.hostCallbackAdapter;
  }

  /**
   * Frees a deferred callback-vtable parameter buffer via the real, always-exported
   * `boltffi_free_deferred_callback_bytes` (`boltffi_core::callback::deferred_buffer` -- the SAME
   * symbol `runtime/cpp`'s `generic_callback.h` dlsyms for its own JSI-independent trampolines).
   * Unlike `readForeignBytes`/`wrapForeignFunction` (genuinely host-specific mechanisms), this is a
   * real crate export reachable through `exports` like any other -- no host adapter needed, mirrors
   * `takeLastErrorMessage`'s own "cross into a real named export directly" precedent. Only a
   * DEFERRED callback slot's encoded parameter (`AbiCallbackMethod::is_deferred_dispatch`) is ever
   * freed this way -- a non-deferred sync slot's buffer is Rust-stack-scoped and freeing it here
   * would double-free once the caller's own stack frame unwinds.
   */
  freeDeferredCallbackBytes(ptr: bigint, len: bigint): void {
    const exportsRecord = this.exports as Record<string, unknown>;
    const freeFn = exportsRecord["boltffi_free_deferred_callback_bytes"] as
      | ((ptr: bigint, len: bigint) => void)
      | undefined;
    if (!freeFn) {
      throw new Error("boltffi: boltffi_free_deferred_callback_bytes export not found");
    }
    freeFn(ptr, len);
  }

  /**
   * Shares the arena's backing `ArrayBuffer` with the host adapter, once now and again every time
   * the arena grows (replaces the buffer) -- `NativeMemoryArena.setOnGrow`'s contract existed but
   * was never wired to anything (Codex finding 3, HIGH growth binding): without this, offsets
   * issued before a grow resolve against the OLD buffer's real native address on the host side
   * forever, silently reading/writing stale (potentially freed, once GC'd) memory once JS moves on
   * to the new one.
   *
   * The C++ contract this binds to (implemented on the runtime/cpp side, not here): the host's
   * JSI HostObject exposes an optional `exports.__boltffi_native_bind_arena(buffer: ArrayBuffer):
   * void` function. It is called once at construction with the arena's initial buffer, and again
   * with the NEW buffer every time `NativeMemoryArena` grows -- the adapter re-derives the real
   * native base address each time (e.g. via `jsi::ArrayBuffer::data()`) and uses it to translate
   * every subsequent arena offset this file hands across the FFI boundary. A host that doesn't
   * expose this function (the Bun scalar-only test harness, or any test construction) is left
   * alone -- this file never requires it.
   */
  private bindArenaToHost(): void {
    const exportsRecord = this.exports as Record<string, unknown>;
    const bindArena = exportsRecord["__boltffi_native_bind_arena"] as
      | ((buffer: ArrayBuffer) => void)
      | undefined;
    if (!bindArena) return;
    bindArena(this.arena.buffer);
    this.arena.setOnGrow((buffer) => bindArena(buffer));
  }

  // ---- scalar scratch (mirrors BoltFFIModule.allocStatus/readStatus/freeStatus, made public
  // under names templates already call directly for struct-return-slot routes:
  // `_module.exports.boltffi_wasm_alloc`/`_free` in wasm mode becomes `_module.allocScratch`/
  // `freeScratch` here, and on `BoltFFIModule` too — see module.ts's matching addition) ----

  allocScratch(size: number): number {
    return this.arena.alloc(size);
  }

  freeScratch(ptr: number, size: number): void {
    this.arena.free(ptr, size);
  }

  /**
   * `complete` is the SYNCHRONOUS dispatch of the native `_complete` export every async method's
   * generated completion route calls this through (`_module.completeAsync((statusPtr) =>
   * _exports.xxx_complete(...))`) -- the one funnel ALL of them share. Brackets JUST that call
   * with the arena's call-in-flight growth guard (`beginCall`/`endCall`): `statusPtr` is an
   * out-param the callee writes through after this function returns control to it, so a
   * synchronous host-callback the completion triggers (reentering JS) must not be allowed to grow
   * the arena and orphan that write -- see `native_arena.ts`'s class doc. Reading the status back
   * out happens AFTER `endCall()`, once the call is no longer in flight and reading is safe again.
   */
  completeAsync<T>(complete: (statusPtr: number) => T): T {
    const statusPtr = this.arena.alloc(4);
    this.arena.dataView.setInt32(statusPtr, 0, true);
    try {
      this.arena.beginCall();
      let result: T;
      try {
        result = complete(statusPtr);
      } finally {
        this.arena.endCall();
      }
      this.checkStatus(this.arena.dataView.getInt32(statusPtr, true));
      return result;
    } finally {
      this.arena.free(statusPtr, 4);
    }
  }

  /** Reads the little-endian `FfiStatus.code` a native `_complete` export wrote into a status
   * buffer -- the instance-method form of the standalone `readNativeStatusCode` (usable directly
   * by name from generated code via `_module.readStatusCode(...)`, matching every other
   * `_module.xxx(...)` call site). */
  readStatusCode(buffer: Uint8Array): number {
    return readNativeStatusCode(buffer);
  }

  /**
   * The ONE place every native-async return route (packed-buffer via `completeAsync` above, AND
   * the void/scalar routes that skip the arena and pass a raw status buffer straight to the
   * native `_complete` export) checks an `FfiStatus` code -- so cancellation/invalid-argument
   * report the same error class regardless of which return route a method happens to use (Codex
   * finding 5, MEDIUM status semantics: before this, the void/scalar branches hand-rolled their
   * own `statusCode !== 0` check inline in the template and always threw a generic `Error`, never
   * `BoltFFICancelledError`, and never recognized status 3 at all). Mirrors wasm's
   * `BoltFFIModule.checkStatus` (module.ts) code-for-code, including status 3's message, so the
   * two backends' error taxonomies never drift apart per status code.
   */
  checkStatus(status: number): void {
    if (status === 0) {
      return;
    }
    if (status === NativeFfiStatus.InvalidArgument) {
      throw new Error("invalid argument");
    }
    if (status === NativeFfiStatus.Cancelled) {
      throw new BoltFFICancelledError();
    }
    throw new Error(`native async call failed with status ${status}`);
  }

  // ---- string/bytes/primitive-array param allocation ----

  allocString(value: string): NativeStringAlloc {
    const encoded = this.encoder.encode(value);
    const ptr = this.arena.alloc(encoded.length);
    this.arena.byteView.set(encoded, ptr);
    return { ptr, len: encoded.length };
  }

  allocBytes(value: Uint8Array): NativeStringAlloc {
    const ptr = this.arena.alloc(value.length);
    this.arena.byteView.set(value, ptr);
    return { ptr, len: value.length };
  }

  freeAlloc(alloc: NativeStringAlloc): void {
    this.arena.free(alloc.ptr, alloc.len);
  }

  private allocTypedArray(
    byteLen: number,
    write: (view: DataView, ptr: number) => void
  ): NativePrimitiveBufferAlloc {
    const ptr = this.arena.alloc(byteLen);
    write(this.arena.dataView, ptr);
    return { ptr, len: byteLen, allocationSize: byteLen };
  }

  allocI8Array(value: Int8Array | readonly number[]): NativePrimitiveBufferAlloc {
    const alloc = this.allocTypedArray(value.length, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setInt8(ptr + i, value[i]);
    });
    return { ...alloc, len: value.length };
  }

  allocU8Array(value: Uint8Array | readonly number[]): NativePrimitiveBufferAlloc {
    const ptr = this.arena.alloc(value.length);
    this.arena.byteView.set(value, ptr);
    return { ptr, len: value.length, allocationSize: value.length };
  }

  allocI16Array(value: Int16Array | readonly number[]): NativePrimitiveBufferAlloc {
    const byteLen = value.length * 2;
    const alloc = this.allocTypedArray(byteLen, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setInt16(ptr + i * 2, value[i], true);
    });
    return { ...alloc, len: value.length };
  }

  allocU16Array(value: Uint16Array | readonly number[]): NativePrimitiveBufferAlloc {
    const byteLen = value.length * 2;
    const alloc = this.allocTypedArray(byteLen, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setUint16(ptr + i * 2, value[i], true);
    });
    return { ...alloc, len: value.length };
  }

  allocI32Array(value: Int32Array | readonly number[]): NativePrimitiveBufferAlloc {
    const byteLen = value.length * 4;
    const alloc = this.allocTypedArray(byteLen, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setInt32(ptr + i * 4, value[i], true);
    });
    return { ...alloc, len: value.length };
  }

  allocU32Array(value: Uint32Array | readonly number[]): NativePrimitiveBufferAlloc {
    const byteLen = value.length * 4;
    const alloc = this.allocTypedArray(byteLen, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setUint32(ptr + i * 4, value[i], true);
    });
    return { ...alloc, len: value.length };
  }

  allocI64Array(value: BigInt64Array | readonly bigint[]): NativePrimitiveBufferAlloc {
    const byteLen = value.length * 8;
    const alloc = this.allocTypedArray(byteLen, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setBigInt64(ptr + i * 8, BigInt(value[i]), true);
    });
    return { ...alloc, len: value.length };
  }

  allocU64Array(value: BigUint64Array | readonly bigint[]): NativePrimitiveBufferAlloc {
    const byteLen = value.length * 8;
    const alloc = this.allocTypedArray(byteLen, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setBigUint64(ptr + i * 8, BigInt(value[i]), true);
    });
    return { ...alloc, len: value.length };
  }

  allocF32Array(value: Float32Array | readonly number[]): NativePrimitiveBufferAlloc {
    const byteLen = value.length * 4;
    const alloc = this.allocTypedArray(byteLen, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setFloat32(ptr + i * 4, value[i], true);
    });
    return { ...alloc, len: value.length };
  }

  allocF64Array(value: Float64Array | readonly number[]): NativePrimitiveBufferAlloc {
    const byteLen = value.length * 8;
    const alloc = this.allocTypedArray(byteLen, (view, ptr) => {
      for (let i = 0; i < value.length; i++) view.setFloat64(ptr + i * 8, value[i], true);
    });
    return { ...alloc, len: value.length };
  }

  allocBoolArray(value: readonly boolean[]): NativePrimitiveBufferAlloc {
    const ptr = this.arena.alloc(value.length);
    for (let i = 0; i < value.length; i++) this.arena.byteView[ptr + i] = value[i] ? 1 : 0;
    return { ptr, len: value.length, allocationSize: value.length };
  }

  allocPrimitiveBuffer(
    value: ReadonlyArray<number | bigint | boolean>,
    elementType: NativePrimitiveBufferElementType
  ): NativePrimitiveBufferAlloc {
    switch (elementType) {
      case "bool":
        return this.allocBoolArray(value as readonly boolean[]);
      case "i8":
        return this.allocI8Array(value as readonly number[]);
      case "u8":
        return this.allocU8Array(value as readonly number[]);
      case "i16":
        return this.allocI16Array(value as readonly number[]);
      case "u16":
        return this.allocU16Array(value as readonly number[]);
      case "i32":
      case "isize":
        return this.allocI32Array(value as readonly number[]);
      case "u32":
      case "usize":
        return this.allocU32Array(value as readonly number[]);
      case "i64":
        return this.allocI64Array(value as readonly bigint[]);
      case "u64":
        return this.allocU64Array(value as readonly bigint[]);
      case "f32":
        return this.allocF32Array(value as readonly number[]);
      case "f64":
        return this.allocF64Array(value as readonly number[]);
    }
  }

  freePrimitiveBuffer(allocation: NativePrimitiveBufferAlloc): void {
    this.arena.free(allocation.ptr, allocation.allocationSize);
  }

  // ---- WireWriter-backed params (StructValue/CodecEncoded/OtherEncoded/CompositeBuffer routes,
  // and host-callback response encoding in callback.txt) ----

  private arenaAllocator(): WasmWireWriterAllocator {
    return {
      alloc: (size) => this.arena.alloc(size),
      realloc: (ptr, oldSize, newSize) => this.arena.realloc(ptr, oldSize, newSize),
      free: (ptr, size) => this.arena.free(ptr, size),
      buffer: () => this.arena.buffer,
    };
  }

  allocWriter(size: number): WireWriter {
    return WireWriter.withWasmAllocation(size, this.arenaAllocator());
  }

  freeWriter(writer: WireWriter): void {
    writer.release();
  }

  allocCompositeBuffer<T>(
    value: readonly T[],
    elementSize: number,
    writeElement: (writer: WireWriter, value: T) => void
  ): WireWriter {
    const writer = this.allocWriter(value.length * elementSize);
    value.forEach((entry) => writeElement(writer, entry));
    return writer;
  }

  // ---- buffer descriptors (packed-return / async-complete plumbing) ----

  allocBufDescriptor(): number {
    return this.arena.alloc(NATIVE_FFI_BUF_DESCRIPTOR_SIZE);
  }

  freeBufDescriptor(ptr: number): void {
    this.arena.free(ptr, NATIVE_FFI_BUF_DESCRIPTOR_SIZE);
  }

  writeBufDescriptor(bufPtr: number, dataPtr: number, dataLen: number, dataCap: number, dataAlign = 1): void {
    const view = this.arena.dataView;
    view.setUint32(bufPtr, dataPtr, true);
    view.setUint32(bufPtr + 4, dataLen, true);
    view.setUint32(bufPtr + 8, dataCap, true);
    view.setUint32(bufPtr + 12, dataAlign, true);
  }

  private readBufDescriptor(bufPtr: number): { ptr: number; len: number; cap: number; align: number } {
    const view = this.arena.dataView;
    return {
      ptr: view.getUint32(bufPtr, true),
      len: view.getUint32(bufPtr + 4, true),
      cap: view.getUint32(bufPtr + 8, true),
      align: view.getUint32(bufPtr + 12, true) || 1,
    };
  }

  readerFromBuf(bufPtr: number): WireReader {
    const { ptr } = this.readBufDescriptor(bufPtr);
    return new WireReader(this.arena.buffer, ptr);
  }

  freeBuf(bufPtr: number): void {
    const { ptr, cap } = this.readBufDescriptor(bufPtr);
    this.arena.free(ptr, cap);
    this.arena.free(bufPtr, NATIVE_FFI_BUF_DESCRIPTOR_SIZE);
  }

  // ---- takeBuf*/takeSlot*/takePacked* decode-and-free families (mirror BoltFFIModule 1:1) ----

  // `takeBuf*`/`takeBufStructArray` below deliberately do NOT free the payload they read --
  // matching `BoltFFIModule.takeBufU8Array`/`takeBufStructArray` (module.ts), which only ever
  // copy bytes out of wasm linear memory. Every call site that decodes a buf descriptor (both
  // `readerFromBuf` for reader-based routes and these `takeBuf*` calls for the direct
  // primitive/struct-array routes -- lower.rs's `direct_vec_output_route` buf_decode table and
  // emit.rs's `composite_buf_decode_expr`) is followed, in the SAME generated `finally` block, by
  // exactly one `_module.freeBuf(bufPtr)` call that frees both the payload and the descriptor
  // together. A `takeBuf*` that also freed the payload here double-frees it once that `freeBuf`
  // runs (regression: two unrelated later allocations could alias the same freed pointer).

  takeBufU8Array(bufPtr: number): Uint8Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    return ptr === 0 ? new Uint8Array(0) : this.arena.byteView.slice(ptr, ptr + len);
  }

  takeBufI8Array(bufPtr: number): Int8Array {
    const bytes = this.takeBufU8Array(bufPtr);
    return new Int8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  private takeBufTyped<T extends ArrayBufferView>(
    bufPtr: number,
    bytesPerElement: number,
    build: (buffer: ArrayBuffer, byteOffset: number, count: number) => T,
    empty: () => T
  ): T {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return empty();
    const count = Math.floor(len / bytesPerElement);
    const copy = this.arena.byteView.slice(ptr, ptr + len);
    return build(copy.buffer, 0, count);
  }

  takeBufI16Array(bufPtr: number): Int16Array {
    return this.takeBufTyped(bufPtr, 2, (b, o, c) => new Int16Array(b, o, c), () => new Int16Array(0));
  }

  takeBufU16Array(bufPtr: number): Uint16Array {
    return this.takeBufTyped(bufPtr, 2, (b, o, c) => new Uint16Array(b, o, c), () => new Uint16Array(0));
  }

  takeBufI32Array(bufPtr: number): Int32Array {
    return this.takeBufTyped(bufPtr, 4, (b, o, c) => new Int32Array(b, o, c), () => new Int32Array(0));
  }

  takeBufU32Array(bufPtr: number): Uint32Array {
    return this.takeBufTyped(bufPtr, 4, (b, o, c) => new Uint32Array(b, o, c), () => new Uint32Array(0));
  }

  takeBufI64Array(bufPtr: number): BigInt64Array {
    return this.takeBufTyped(bufPtr, 8, (b, o, c) => new BigInt64Array(b, o, c), () => new BigInt64Array(0));
  }

  takeBufU64Array(bufPtr: number): BigUint64Array {
    return this.takeBufTyped(bufPtr, 8, (b, o, c) => new BigUint64Array(b, o, c), () => new BigUint64Array(0));
  }

  takeBufF32Array(bufPtr: number): Float32Array {
    return this.takeBufTyped(bufPtr, 4, (b, o, c) => new Float32Array(b, o, c), () => new Float32Array(0));
  }

  takeBufF64Array(bufPtr: number): Float64Array {
    return this.takeBufTyped(bufPtr, 8, (b, o, c) => new Float64Array(b, o, c), () => new Float64Array(0));
  }

  takeBufBoolArray(bufPtr: number): boolean[] {
    const bytes = this.takeBufU8Array(bufPtr);
    return Array.from(bytes, (value) => value !== 0);
  }

  takeBufStructArray<T>(bufPtr: number, stride: number, decode: (view: DataView, offset: number) => T): T[] {
    const { ptr, len: byteLen } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return [];
    const copy = this.arena.byteView.slice(ptr, ptr + byteLen);
    const view = new DataView(copy.buffer, copy.byteOffset, copy.byteLength);
    const count = Math.floor(byteLen / stride);
    return Array.from({ length: count }, (_, index) => decode(view, index * stride));
  }

  // ---- return-slot family (struct-return-slot routes; reserved offset 0 in the arena) ----

  private readReturnSlot(): { ptr: number; len: number; cap: number; align: number } {
    const view = this.arena.dataView;
    const addr = this.arena.returnSlotAddr;
    return {
      ptr: view.getUint32(addr, true),
      len: view.getUint32(addr + 4, true),
      cap: view.getUint32(addr + 8, true),
      align: view.getUint32(addr + 12, true) || 1,
    };
  }

  takeSlotU8Array(): Uint8Array {
    const { ptr, len, cap } = this.readReturnSlot();
    if (ptr === 0) return new Uint8Array(0);
    const result = this.arena.byteView.slice(ptr, ptr + len);
    this.arena.free(ptr, cap || len);
    return result;
  }

  takeSlotI8Array(): Int8Array {
    const bytes = this.takeSlotU8Array();
    return new Int8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  private takeSlotTyped<T extends ArrayBufferView>(
    bytesPerElement: number,
    build: (buffer: ArrayBuffer, byteOffset: number, count: number) => T,
    empty: () => T
  ): T {
    const { ptr, len, cap } = this.readReturnSlot();
    if (ptr === 0) return empty();
    const count = Math.floor(len / bytesPerElement);
    const copy = this.arena.byteView.slice(ptr, ptr + len);
    this.arena.free(ptr, cap || len);
    return build(copy.buffer, 0, count);
  }

  takeSlotI16Array(): Int16Array {
    return this.takeSlotTyped(2, (b, o, c) => new Int16Array(b, o, c), () => new Int16Array(0));
  }

  takeSlotU16Array(): Uint16Array {
    return this.takeSlotTyped(2, (b, o, c) => new Uint16Array(b, o, c), () => new Uint16Array(0));
  }

  takeSlotI32Array(): Int32Array {
    return this.takeSlotTyped(4, (b, o, c) => new Int32Array(b, o, c), () => new Int32Array(0));
  }

  takeSlotU32Array(): Uint32Array {
    return this.takeSlotTyped(4, (b, o, c) => new Uint32Array(b, o, c), () => new Uint32Array(0));
  }

  takeSlotI64Array(): BigInt64Array {
    return this.takeSlotTyped(8, (b, o, c) => new BigInt64Array(b, o, c), () => new BigInt64Array(0));
  }

  takeSlotU64Array(): BigUint64Array {
    return this.takeSlotTyped(8, (b, o, c) => new BigUint64Array(b, o, c), () => new BigUint64Array(0));
  }

  takeSlotF32Array(): Float32Array {
    return this.takeSlotTyped(4, (b, o, c) => new Float32Array(b, o, c), () => new Float32Array(0));
  }

  takeSlotF64Array(): Float64Array {
    return this.takeSlotTyped(8, (b, o, c) => new Float64Array(b, o, c), () => new Float64Array(0));
  }

  takeSlotBoolArray(): boolean[] {
    const bytes = this.takeSlotU8Array();
    return Array.from(bytes, (value) => value !== 0);
  }

  takeSlotStructArray<T>(stride: number, decode: (view: DataView, offset: number) => T): T[] {
    const { ptr, len: byteLen, cap } = this.readReturnSlot();
    if (ptr === 0) return [];
    const copy = this.arena.byteView.slice(ptr, ptr + byteLen);
    this.arena.free(ptr, cap || byteLen);
    const view = new DataView(copy.buffer, copy.byteOffset, copy.byteLength);
    const count = Math.floor(byteLen / stride);
    return Array.from({ length: count }, (_, index) => decode(view, index * stride));
  }

  // ---- packed (NaN-boxed bigint pointer|length) family ----

  private unpackPacked(packed: bigint): { pointer: number; length: number } {
    return {
      pointer: Number(packed & 0xffff_ffffn),
      length: Number((packed >> 32n) & 0xffff_ffffn),
    };
  }

  private takePackedTyped<T extends ArrayBufferView>(
    packed: bigint,
    bytesPerElement: number,
    build: (buffer: ArrayBuffer, byteOffset: number, count: number) => T,
    empty: () => T
  ): T {
    const { pointer, length } = this.unpackPacked(packed);
    if (pointer === 0 || length === 0) return empty();
    const count = Math.floor(length / bytesPerElement);
    const copy = this.arena.byteView.slice(pointer, pointer + length);
    this.arena.free(pointer, length);
    return build(copy.buffer, 0, count);
  }

  takePackedUtf8String(packed: bigint): string {
    const { pointer, length } = this.unpackPacked(packed);
    if (pointer === 0 || length === 0) return "";
    const bytes = this.arena.byteView.slice(pointer, pointer + length);
    this.arena.free(pointer, length);
    return this.decoder.decode(bytes);
  }

  takePackedBuffer(packed: bigint): WireReader {
    const { pointer, length } = this.unpackPacked(packed);
    if (pointer === 0 || length === 0) {
      return new WireReader(new ArrayBuffer(0), 0);
    }
    const copy = this.arena.byteView.slice(pointer, pointer + length);
    this.arena.free(pointer, length);
    return new WireReader(copy.buffer, 0);
  }

  takePackedI8Array(packed: bigint): Int8Array {
    const bytes = this.takePackedTyped(packed, 1, (b, o, c) => new Uint8Array(b, o, c), () => new Uint8Array(0));
    return new Int8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  takePackedU8Array(packed: bigint): Uint8Array {
    return this.takePackedTyped(packed, 1, (b, o, c) => new Uint8Array(b, o, c), () => new Uint8Array(0));
  }

  takePackedI16Array(packed: bigint): Int16Array {
    return this.takePackedTyped(packed, 2, (b, o, c) => new Int16Array(b, o, c), () => new Int16Array(0));
  }

  takePackedU16Array(packed: bigint): Uint16Array {
    return this.takePackedTyped(packed, 2, (b, o, c) => new Uint16Array(b, o, c), () => new Uint16Array(0));
  }

  takePackedI32Array(packed: bigint): Int32Array {
    return this.takePackedTyped(packed, 4, (b, o, c) => new Int32Array(b, o, c), () => new Int32Array(0));
  }

  takePackedU32Array(packed: bigint): Uint32Array {
    return this.takePackedTyped(packed, 4, (b, o, c) => new Uint32Array(b, o, c), () => new Uint32Array(0));
  }

  takePackedI64Array(packed: bigint): BigInt64Array {
    return this.takePackedTyped(packed, 8, (b, o, c) => new BigInt64Array(b, o, c), () => new BigInt64Array(0));
  }

  takePackedU64Array(packed: bigint): BigUint64Array {
    return this.takePackedTyped(packed, 8, (b, o, c) => new BigUint64Array(b, o, c), () => new BigUint64Array(0));
  }

  takePackedF32Array(packed: bigint): Float32Array {
    return this.takePackedTyped(packed, 4, (b, o, c) => new Float32Array(b, o, c), () => new Float32Array(0));
  }

  takePackedF64Array(packed: bigint): Float64Array {
    return this.takePackedTyped(packed, 8, (b, o, c) => new Float64Array(b, o, c), () => new Float64Array(0));
  }

  private takePackedOptionalPrimitive<T>(
    packed: bigint,
    encodedSize: number,
    readValue: (view: DataView, valueOffset: number) => T
  ): T | null {
    const { pointer, length } = this.unpackPacked(packed);
    if (pointer === 0 || length === 0) return null;
    const view = this.arena.dataView;
    const tag = view.getUint8(pointer);
    if (tag === 0) {
      this.arena.free(pointer, length);
      return null;
    }
    if (length < 1 + encodedSize) {
      this.arena.free(pointer, length);
      throw new Error("Invalid packed optional payload");
    }
    const value = readValue(view, pointer + 1);
    this.arena.free(pointer, length);
    return value;
  }

  takePackedOptionalBool(packed: bigint): boolean | null {
    return this.takePackedOptionalPrimitive(packed, 1, (view, offset) => view.getUint8(offset) !== 0);
  }

  takePackedOptionalI8(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 1, (view, offset) => view.getInt8(offset));
  }

  takePackedOptionalU8(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 1, (view, offset) => view.getUint8(offset));
  }

  takePackedOptionalI16(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 2, (view, offset) => view.getInt16(offset, true));
  }

  takePackedOptionalU16(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 2, (view, offset) => view.getUint16(offset, true));
  }

  takePackedOptionalI32(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 4, (view, offset) => view.getInt32(offset, true));
  }

  takePackedOptionalU32(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 4, (view, offset) => view.getUint32(offset, true));
  }

  takePackedOptionalI64(packed: bigint): bigint | null {
    return this.takePackedOptionalPrimitive(packed, 8, (view, offset) => view.getBigInt64(offset, true));
  }

  takePackedOptionalU64(packed: bigint): bigint | null {
    return this.takePackedOptionalPrimitive(packed, 8, (view, offset) => view.getBigUint64(offset, true));
  }

  takePackedOptionalF32(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 4, (view, offset) => view.getFloat32(offset, true));
  }

  takePackedOptionalF64(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 8, (view, offset) => view.getFloat64(offset, true));
  }

  unpackOptionBool(packed: number): boolean | null {
    return Number.isNaN(packed) ? null : packed !== 0;
  }

  unpackOptionI8(packed: number): number | null {
    return Number.isNaN(packed) ? null : packed | 0;
  }

  unpackOptionU8(packed: number): number | null {
    return Number.isNaN(packed) ? null : packed >>> 0;
  }

  unpackOptionI16(packed: number): number | null {
    return Number.isNaN(packed) ? null : packed | 0;
  }

  unpackOptionU16(packed: number): number | null {
    return Number.isNaN(packed) ? null : packed >>> 0;
  }

  unpackOptionI32(packed: number): number | null {
    return Number.isNaN(packed) ? null : packed | 0;
  }

  unpackOptionU32(packed: number): number | null {
    return Number.isNaN(packed) ? null : packed >>> 0;
  }

  unpackOptionF32(packed: number): number | null {
    return Number.isNaN(packed) ? null : packed;
  }

  unpackOptionF64(packed: number): number | null {
    return Number.isNaN(packed) ? null : packed;
  }

  // ---- misc host-boundary helpers ----

  readFromMemory(ptr: number, len: number): Uint8Array {
    return this.arena.byteView.slice(ptr, ptr + len);
  }

  writeToMemory(ptr: number, data: Uint8Array): void {
    this.arena.byteView.set(data, ptr);
  }

  /**
   * Reads and clears the thread-local last-error message set by a failed fallible call whose
   * success payload is a handle (e.g. a `Result<Self, Error>` constructor) — see
   * `BoltFFIModule.takeLastErrorMessage`'s doc for the wasm equivalent this mirrors. This is the
   * one method that genuinely crosses into the host's REAL native symbols
   * (`boltffi_last_error_message`/`boltffi_free_string`) rather than staying arena-only: there is
   * no arena-side equivalent of "ask Rust for its thread-local error." Passes an arena offset as
   * the out-param pointer, exactly like wasm passes a linear-memory offset — a real JSI adapter
   * resolves it against the arena's shared backing buffer.
   *
   * Both native calls below (`lastErrorMessage` writing the ptr/len out-param, `freeString`
   * releasing it) are each bracketed with their OWN `beginCall`/`endCall` pair -- two separate
   * dispatches, so two separate marks, never one held open across both (see `native_arena.ts`'s
   * class doc for why any window a synchronous host-callback could reenter through needs the
   * guard, and why it must never span more than the ONE call it protects).
   */
  takeLastErrorMessage(): string {
    const exportsRecord = this.exports as Record<string, unknown>;
    const lastErrorMessage = exportsRecord["boltffi_last_error_message"] as
      | ((outPtr: number) => void)
      | undefined;
    const freeString = exportsRecord["boltffi_free_string"] as ((ptr: number) => void) | undefined;
    if (!lastErrorMessage || !freeString) {
      return "";
    }
    const outPtr = this.arena.alloc(NATIVE_FFI_STRING_SIZE);
    try {
      this.arena.byteView.fill(0, outPtr, outPtr + NATIVE_FFI_STRING_SIZE);
      this.arena.beginCall();
      try {
        lastErrorMessage(outPtr);
      } finally {
        this.arena.endCall();
      }
      const view = this.arena.dataView;
      const strPtr = view.getUint32(outPtr, true);
      const strLen = view.getUint32(outPtr + 4, true);
      const message =
        strPtr === 0 || strLen === 0
          ? ""
          : this.decoder.decode(this.arena.byteView.subarray(strPtr, strPtr + strLen));
      this.arena.beginCall();
      try {
        freeString(outPtr);
      } finally {
        this.arena.endCall();
      }
      return message;
    } finally {
      this.arena.free(outPtr, NATIVE_FFI_STRING_SIZE);
    }
  }
}

export function instantiateBoltFFINative<Exports, Token = unknown>(
  exports: Exports,
  createContinuationTrampoline: NativeTrampolineFactory<Token>,
  callbackHost?: NativeCallbackHostAdapter<Token>
): NativeBoltFFIModule<Exports, Token> {
  return new NativeBoltFFIModule<Exports, Token>(
    exports,
    createContinuationTrampoline,
    new NativeMemoryArena(),
    callbackHost
  );
}
