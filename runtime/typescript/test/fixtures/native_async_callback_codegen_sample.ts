// A literal transcription of `callback.txt`'s `native_async` branch output for an async callback
// TRAIT method's vtable slot (adversarial-review finding 2's fix) -- the exact shape
// `boltffi_bindgen/src/render/typescript/lower.rs`'s
// `native_async_wire_result_dispatch_awaits_impl_and_reports_errors_through_completion` test
// asserts as substrings. Proves the emitted dispatch (the `async () => { try { ... } catch (e) {
// ... } }` IIFE, its `wrapForeignFunction` completion-pointer shape, and the error-path wire
// encoding) actually TYPE-CHECKS against the real `@boltffi/runtime` types, not just that the
// right substrings appear in the rendered text -- mirrors `native_async_codegen_sample.ts`'s own
// "literal transcription checked ad hoc" convention.
// Not wired into any build; checked ad hoc via `npm run test:codegen-sample`.
import type { NativeBoltFFIModule } from "../../src/native.js";

interface AsyncKvExports {
  boltffi_register_callback_async_kv: (ptr: unknown) => void;
}

declare const _module: NativeBoltFFIModule<AsyncKvExports, unknown>;
declare const _exports: AsyncKvExports;

interface AsyncKv {
  get(): Promise<string>;
}

declare function _async_kv_lookup(handle: number): AsyncKv;

// The exact per-slot closure `native_async_method_vtable_slot` emits for a wire-encoded async
// result (`AsyncKv::get() -> String`), reproduced verbatim from the Rust-side test's assertions.
const getSlot = (
  handle: bigint,
  __complete: bigint,
  __userdata: bigint
): void => {
  const complete = _module.callbackHost.wrapForeignFunction(__complete, {
    args: ["u64", "ptr", "u64", "i32"],
    returns: "void",
  });
  (async () => {
    try {
      const impl = _async_kv_lookup(Number(handle));
      const result = await impl.get();
      const writer = _module.allocWriter(32);
      writer.writeString(result);
      complete(__userdata, BigInt(writer.ptr), BigInt(writer.len), 0n);
      _module.freeWriter(writer);
    } catch (e) {
      const __errMsg = e instanceof Error ? e.message : String(e);
      const __errWriter = _module.allocWriter(4 + __errMsg.length * 3);
      __errWriter.writeString(__errMsg);
      complete(__userdata, BigInt(__errWriter.ptr), BigInt(__errWriter.len), 100n);
      _module.freeWriter(__errWriter);
    }
  })();
};

export function __registerAsyncKvGetSlotForTypeCheckOnly(): typeof getSlot {
  return getSlot;
}
