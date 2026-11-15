import { expect, it } from "vitest";
import { StreamRevealTimeline } from "./StreamRevealTimeline";

it("retargets the pending suffix for 300 ms without moving its frontier backwards", () => {
  const reveal = new StreamRevealTimeline("甲乙丙", true, 0);
  expect(reveal.position(150)).toBe(1.5);
  reveal.update("甲乙丙丁戊己", true, 150);
  expect(reveal.position(150)).toBe(1.5);
  expect(reveal.position(300)).toBe(3.75);
  reveal.update("甲乙丙丁戊己庚辛壬", true, 300);
  expect(reveal.position(300)).toBe(3.75);
  expect(reveal.position(450)).toBe(6.375);
  expect(reveal.position(599)).toBeLessThan(9);
  expect(reveal.position(600)).toBe(9);
});
it("finishes the last fade after streaming ends without restarting the clock", () => {
  const reveal = new StreamRevealTimeline("甲乙丙", true, 0);
  reveal.update("甲乙丙", false, 100);
  expect(reveal.position(150)).toBe(1.5);
  expect(reveal.position(300)).toBe(3);
});
it("shows history, corrections and reduced motion immediately", () => {
  const history = new StreamRevealTimeline("历史消息", false, 0);
  expect(history.position(0)).toBe(4);
  history.update("更新历史", false, 10);
  expect(history.position(10)).toBe(4);
  const stream = new StreamRevealTimeline("旧文字", true, 0);
  stream.update("新文字", true, 100);
  expect(stream.position(100)).toBe(3);
  stream.update("新文字继续", true, 150);
  stream.settle();
  expect(stream.position(150)).toBe(5);
});

it("changes the configured duration from its current frontier, and accepts zero", () => {
  const reveal = new StreamRevealTimeline("甲乙丙丁", true, 0, 1000);
  expect(reveal.position(200)).toBe(.8);
  reveal.update("甲乙丙丁", true, 200, 600);
  expect(reveal.position(200)).toBe(.8);
  expect(reveal.position(500)).toBeCloseTo(2.4);
  expect(reveal.position(800)).toBe(4);
  reveal.update("甲乙丙丁戊", true, 800, 0);
  expect(reveal.position(800)).toBe(5);
});
