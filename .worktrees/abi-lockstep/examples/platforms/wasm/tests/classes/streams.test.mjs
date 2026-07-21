import { assert, demo } from "../support/index.mjs";

export async function run() {
  const bus = demo.EventBus.new();
  bus.emitValue(1);
  assert.equal(bus.emitBatch([2, 3, 4]), 3);
  bus.emitPoint({ x: 1, y: 2 });
  bus.emitPoint({ x: 3, y: 4 });
  bus.emitMessage({ text: "alpha", values: [1, 2] });
  bus.dispose();
}
