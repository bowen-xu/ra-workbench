import { expect, it } from "vitest";
import { WritingDemoTextStream, type WritingDemoTextPacing } from "./WritingDemoTextStream";

it("keeps the emitted cursor when chunk size and interval change midstream", () => {
  let pacing: WritingDemoTextPacing = { charactersPerChunk: 2, intervalMs: 100 };
  const stream = new WritingDemoTextStream("甲😀乙丙丁", () => pacing);
  expect(stream.next()).toBe("甲😀");
  pacing = { charactersPerChunk: 1, intervalMs: 500 };
  expect(stream.remainingSteps).toBe(3);
  expect(stream.intervalMs).toBe(500);
  expect(stream.next()).toBe("乙");
  pacing = { charactersPerChunk: 5, intervalMs: 50 };
  expect(stream.next()).toBe("丙丁");
  expect(stream.remainingSteps).toBe(0);
});
