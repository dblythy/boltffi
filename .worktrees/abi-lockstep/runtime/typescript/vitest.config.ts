import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // native.bun.test.ts drives the real native backend through Bun's own FFI (`bun:ffi`,
    // `bun:test`) against a real compiled dylib -- it requires the Bun runtime itself and a
    // Rust toolchain, neither of which vitest's Node-based collection can satisfy. Run it
    // explicitly via `bun test test/native.bun.test.ts`; vitest's default suite stays wasm-only.
    exclude: ["**/node_modules/**", "**/dist/**", "**/*.bun.test.ts"],
  },
});
