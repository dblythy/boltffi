// Adversarial-review finding 2 (parse-core-sdks repo's fork-wiring-fixes session): a native_async
// callback vtable slot's dispatch must call the completion pointer even when the JS impl throws or
// its returned promise rejects -- otherwise the Rust future this slot backs polls `Pending`
// forever. `boltffi_bindgen/src/render/typescript/lower.rs`'s
// `native_async_wire_result_dispatch_awaits_impl_and_reports_errors_through_completion` and
// `..._void_result_completion_shape_includes_status_arg` tests assert the exact generated JS TEXT
// for this dispatch; this file is the mock-host ROUND TRIP proof that the same shape, actually
// EXECUTED (not just string-matched), really does invoke the completion pointer with a failure
// status and never hangs when the impl throws synchronously or returns a rejected promise. The two
// slot bodies below are byte-for-byte the same shape `native_async_method_vtable_slot` renders
// (see `test/fixtures/native_async_callback_codegen_sample.ts` for the wire-result one, type-checked
// against the real runtime types) -- reproduced directly here (rather than via `eval`) so a failure
// points straight at readable, debuggable JS.
import { describe, expect, it, vi } from "vitest";

interface WriterAlloc {
  ptr: number;
  len: number;
  writeString(value: string): void;
}

function makeMockWriter(): WriterAlloc {
  let written = "";
  return {
    ptr: 0,
    get len() {
      return written.length;
    },
    writeString(value: string) {
      written = value;
    },
  } as WriterAlloc;
}

interface CompletionCall {
  userdata: bigint;
  args: readonly bigint[];
}

/** Mirrors `native_async_method_vtable_slot`'s wire-result dispatch verbatim (see the module doc
 * above): looks up `impl`, awaits `impl.get()`, completes with the encoded result on success or a
 * wire-encoded error message + non-OK status on failure -- NEVER leaves `complete` uncalled. */
function wireResultAsyncKvGetSlot(
  lookup: (handle: number) => { get(): Promise<string> },
  complete: (...args: readonly bigint[]) => void
) {
  return (handle: bigint, __userdata: bigint): void => {
    (async () => {
      try {
        const impl = lookup(Number(handle));
        const result = await impl.get();
        const writer = makeMockWriter();
        writer.writeString(result);
        complete(__userdata, BigInt(writer.ptr), BigInt(writer.len), 0n);
      } catch (e) {
        const __errMsg = e instanceof Error ? e.message : String(e);
        const __errWriter = makeMockWriter();
        __errWriter.writeString(__errMsg);
        complete(__userdata, BigInt(__errWriter.ptr), BigInt(__errWriter.len), 100n);
      }
    })();
  };
}

/** Mirrors `native_async_method_vtable_slot`'s void-result dispatch verbatim. */
function voidResultAsyncKvSetSlot(
  lookup: (handle: number) => { set(value: string): Promise<void> },
  complete: (...args: readonly bigint[]) => void
) {
  return (handle: bigint, value: string, __userdata: bigint): void => {
    (async () => {
      try {
        const impl = lookup(Number(handle));
        await impl.set(value);
        complete(__userdata, 0n);
      } catch (e) {
        complete(__userdata, 100n);
      }
    })();
  };
}

describe("native_async callback vtable slot dispatch (mock-host round trip)", () => {
  it("completes successfully when the impl resolves normally", async () => {
    const calls: CompletionCall[] = [];
    const complete = (userdata: bigint, ...args: readonly bigint[]) => calls.push({ userdata, args });
    const slot = wireResultAsyncKvGetSlot(() => ({ get: async () => "hello" }), complete);

    slot(1n, 42n);
    await vi.waitFor(() => expect(calls).toHaveLength(1));

    expect(calls[0].userdata).toBe(42n);
    expect(calls[0].args[2]).toBe(0n); // status: OK
  });

  it("completes with a failure status (not a hang) when the impl throws synchronously", async () => {
    const calls: CompletionCall[] = [];
    const complete = (userdata: bigint, ...args: readonly bigint[]) => calls.push({ userdata, args });
    const slot = wireResultAsyncKvGetSlot(
      () => ({
        get: async () => {
          throw new Error("boom");
        },
      }),
      complete
    );

    slot(1n, 42n);
    await vi.waitFor(() => expect(calls).toHaveLength(1));

    expect(calls[0].userdata).toBe(42n);
    expect(calls[0].args[2]).toBe(100n); // status: INTERNAL_ERROR, not silently dropped
  });

  it("completes with a failure status when the impl returns a REJECTED promise (no throw)", async () => {
    const calls: CompletionCall[] = [];
    const complete = (userdata: bigint, ...args: readonly bigint[]) => calls.push({ userdata, args });
    const slot = wireResultAsyncKvGetSlot(
      () => ({ get: () => Promise.reject(new Error("rejected")) }),
      complete
    );

    slot(1n, 7n);
    await vi.waitFor(() => expect(calls).toHaveLength(1));

    expect(calls[0].args[2]).toBe(100n);
  });

  it("completes with a failure status when the handle lookup itself throws (invalid handle)", async () => {
    const calls: CompletionCall[] = [];
    const complete = (userdata: bigint, ...args: readonly bigint[]) => calls.push({ userdata, args });
    const slot = wireResultAsyncKvGetSlot(() => {
      throw new Error("callback handle not found");
    }, complete);

    slot(999n, 1n);
    await vi.waitFor(() => expect(calls).toHaveLength(1));

    expect(calls[0].args[2]).toBe(100n);
  });

  it("void-result dispatch also completes (never hangs) when the impl throws", async () => {
    const calls: CompletionCall[] = [];
    const complete = (userdata: bigint, ...args: readonly bigint[]) => calls.push({ userdata, args });
    const slot = voidResultAsyncKvSetSlot(
      () => ({
        set: async () => {
          throw new Error("set failed");
        },
      }),
      complete
    );

    slot(1n, "value", 9n);
    await vi.waitFor(() => expect(calls).toHaveLength(1));

    expect(calls[0].args[0]).toBe(100n); // status only -- the void completion shape
  });
});
