/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentMarkdown } from "./AgentMarkdown";
import { StreamRevealDurationProvider } from "../../components/StreamReveal/StreamReveal";
import { writingDemoMarkdownMessages } from "../../../app/previews/agent/WritingDemoMarkdownMessages";

let host: HTMLDivElement, root: Root, now: number;
let nextFrame: number;
let frames: Map<number, FrameRequestCallback>;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  now = 0; nextFrame = 0; frames = new Map();
  vi.spyOn(window.performance, "now").mockImplementation(() => now);
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frames.set(++nextFrame, callback); return nextFrame; });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => { frames.delete(id); });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });
async function render(value: string, streaming = true, durationMs = 300) {
  await act(async () => root.render(<StreamRevealDurationProvider durationMs={durationMs}>
    <AgentMarkdown value={value} streaming={streaming} />
  </StreamRevealDurationProvider>));
  await act(async () => {await vi.advanceTimersByTimeAsync(80);});
}
function tick(time: number): void {
  now = time;
  const callbacks = [...frames.values()]; frames.clear();
  for (const callback of callbacks) callback(time);
}
const glyphs = () => [...host.querySelectorAll<HTMLElement>(".stream-reveal-glyph")];
const progress = (index: number) => Number(glyphs()[index]!.style.getPropertyValue("--stream-reveal-progress"));

it("keeps full selectable text and smoothly extends an unfinished reveal", async () => {
  await render("");
  await render("甲乙丙");
  expect(host.textContent).toBe("甲乙丙");
  expect(glyphs()).toHaveLength(3);
  expect(progress(0)).toBe(0);
  tick(150);
  expect(progress(0)).toBe(1); expect(progress(1)).toBe(.5); expect(progress(2)).toBe(0);
  await render("甲乙丙丁戊己");
  expect(progress(1)).toBe(.5);
  tick(300);
  expect(progress(2)).toBe(1); expect(progress(3)).toBe(.75); expect(progress(4)).toBe(0);
  await render("甲乙丙丁戊己庚辛壬");
  expect(progress(3)).toBe(.75);
  tick(600);
  expect(glyphs().every(node => node.style.getPropertyValue("--stream-reveal-progress") === "1")).toBe(true);
  expect(host.firstElementChild?.getAttribute("data-stream-revealing")).toBe("false");
  expect(frames.size).toBe(0);
});
it("keeps graphemes intact and lets the final 300 ms fade finish", async () => {
  await render("");
  await render("👩‍🔬é中文");
  expect(glyphs().map(node => node.textContent)).toEqual(["👩‍🔬", "é", "中", "文"]);
  tick(100); await render("👩‍🔬é中文", false);
  expect(host.firstElementChild?.getAttribute("data-stream-revealing")).toBe("true");
  tick(300);
  expect(host.firstElementChild?.getAttribute("data-stream-revealing")).toBe("false");
});
it("applies scoped duration changes without replaying revealed text", async () => {
  await render("");
  await render("甲乙丙", true, 600);
  tick(300);
  expect(progress(0)).toBe(1); expect(progress(1)).toBe(.5);
  await render("甲乙丙", true, 900);
  expect(progress(1)).toBe(.5);
  tick(750);
  expect(progress(0)).toBe(1); expect(progress(1)).toBe(1); expect(progress(2)).toBe(.25);
  tick(1200);
  expect(progress(2)).toBe(1);
  await render("甲乙丙丁", true, 0);
  expect(progress(3)).toBe(1); expect(frames.size).toBe(0);
});
it("preserves Markdown tables, math and code without duplicating contents", async () => {
  for (const value of writingDemoMarkdownMessages) {
    await render(value); tick(now + 300);
  }
  expect(host.querySelector('pre code')?.textContent).toBe("values = [1, 2, 3]\nmean = sum(values) / len(values)\nprint(mean)\n");
  expect(host.querySelectorAll('input:checked')).toHaveLength(2);
  await render(writingDemoMarkdownMessages[1]);
  expect(host.querySelectorAll('table tbody tr')).toHaveLength(2);
  expect(host.querySelector('table tbody td')?.textContent).toBe("关键词数量");
  await render(writingDemoMarkdownMessages[2]);
  expect(host.querySelectorAll('.katex')).toHaveLength(8);
  expect(host.querySelector('.katex-mathml .stream-reveal-glyph')).toBeNull();
  expect(host.querySelector('.stream-reveal-glyph.is-block .katex-display')).not.toBeNull();
  expect(host.querySelector('.katex-error')).toBeNull();
});
it("shows historical text and reduced-motion streams without animation, and cancels on unmount", async () => {
  await render("历史消息", false);
  expect(glyphs()).toHaveLength(0); expect(frames.size).toBe(0);
  const original = window.matchMedia.bind(window);
  vi.spyOn(window, "matchMedia").mockImplementation(query => {
    const media = original(query);
    Object.defineProperty(media, "matches", { value: true });
    return media;
  });
  await render("历史消息新增");
  expect(progress(5)).toBe(1); expect(frames.size).toBe(0);
  vi.mocked(window.matchMedia).mockRestore();
  await render("历史消息新增更多");
  expect(frames.size).toBe(1);
  await act(async () => root.unmount());
  expect(frames.size).toBe(0);
});

it("keeps completed backslash formulas atomic while streaming", async () => {
  await render(String.raw`文字 \(x_i`);
  expect(host.querySelector(".katex")).toBeNull();
  await render("文字 \\(x_i\\)\n\n\\[\ny^2\n\\]");
  expect(host.querySelectorAll(".katex")).toHaveLength(2);
  expect(host.querySelector(".stream-reveal-glyph.is-atomic .katex")).not.toBeNull();
  expect(host.querySelector(".stream-reveal-glyph.is-block .katex-display")).not.toBeNull();
  expect(host.querySelector(".katex-mathml .stream-reveal-glyph")).toBeNull();
});
