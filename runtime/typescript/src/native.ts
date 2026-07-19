// The native BoltFFIModule backend (docs/tracks/react-native.md, parse-core-sdks repo, stages
// 1-2). Where module.ts drives generated code against `WebAssembly.Instance.exports` and wasm
// linear memory, this file drives the SAME generated-code shape against a real native ABI the
// host has already dlopen'd/linked (Bun FFI in this fork's own test harness today; a JSI
// HostObject eventually) — no wasm import object, no linear-memory accessors, no growth
// invalidation to defend against (a native pointer is just an address; see the design doc's
// "native's internals are strictly simpler" finding).
//
// What's deliberately NOT here yet (flagged, not silently missing): decoding a wire-encoded
// buffer/struct return (`FfiBuf`, `#[data]` records, `ParseValue`) crossing the native ABI.
// Every async/sync C symbol a native host calls returns either a scalar (i32/i64/bool/handle) or,
// for a small (<=16-byte, all-integer-class) `#[repr(C)]` struct such as `FfiStatus` or
// `BoltFFICallbackHandle`, a shape that decomposes losslessly into that many scalar
// registers/arguments (verified against a real compiled dylib+disassembly-equivalent generated
// C# P/Invoke signatures for this session's PoC fixture — see
// `test/fixtures/rn_poc`). A byte-buffer-valued return needs a real memory accessor (the design
// doc's seam items 2/2b/3) that a raw JS FFI binding (Bun, and Node/Deno equivalents) cannot
// received directly for a *returned* struct larger than two registers — only a native C/C++ layer
// (the stage-3 JSI adapter) can decode that on the native side and hand JS a plain ArrayBuffer.
// Scoping this file to scalar/small-struct shapes is not an oversight; it is the honest boundary
// of what a stage-1/2 slice proves before that adapter exists.

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
 */
export class NativeAsyncFutureManager<Token = unknown> {
  private readonly trampoline: Token;
  private readonly pending = new Map<bigint, NativePendingFuture<Token>>();
  private nextCallbackData = 1n;

  constructor(createTrampoline: NativeTrampolineFactory<Token>) {
    this.trampoline = createTrampoline((callbackData, signal) => this.onSignal(callbackData, signal));
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
      poll(handle, callbackData, this.trampoline);
    });
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
      // Re-issue poll() directly, on whatever thread this callback fired on — no microtask
      // deferral. The design doc's own PoC proved this is the correct first cut (registration is
      // cheap/thread-safe; deferring would only add latency, not safety).
      entry.poll(entry.handle, callbackData, this.trampoline);
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
  Cancelled = 4,
}

/**
 * A native-backend `BoltFFIModule` counterpart (seam item 1: "a parallel entry point... that
 * builds a BoltFFIModule-shaped object from whatever the JSI host object exposes, not from
 * `instance.exports`"). Deliberately thin: unlike `instantiateBoltFFI`, there is no module to
 * instantiate — the host has already dlopen'd/statically linked the real library and bound each
 * symbol however its own FFI layer requires; this just pairs those bound exports with a
 * `NativeAsyncFutureManager` the same way `BoltFFIModule` pairs wasm exports with the wasm
 * `AsyncFutureManager`. No ABI-version check (wasm's `boltffi_wasm_abi_version` export has no
 * native equivalent — a statically-linked/vendored native artifact is version-paired at build
 * time, not loaded from an arbitrary blob at runtime the way a `.wasm` file is).
 */
export interface NativeBoltFFIModule<Exports, Token = unknown> {
  readonly exports: Exports;
  readonly asyncManager: NativeAsyncFutureManager<Token>;
}

export function instantiateBoltFFINative<Exports, Token = unknown>(
  exports: Exports,
  createContinuationTrampoline: NativeTrampolineFactory<Token>
): NativeBoltFFIModule<Exports, Token> {
  return {
    exports,
    asyncManager: new NativeAsyncFutureManager<Token>(createContinuationTrampoline),
  };
}
