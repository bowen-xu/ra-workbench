/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { StreamReveal } from "./StreamReveal";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const animateDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "animate");
afterEach(() => {
  vi.restoreAllMocks();
  if (animateDescriptor) Object.defineProperty(HTMLElement.prototype, "animate", animateDescriptor);
  else Reflect.deleteProperty(HTMLElement.prototype, "animate");
});
function animation() {
  const animate = vi.fn(() => ({cancel:vi.fn()}) as unknown as Animation);
  Object.defineProperty(HTMLElement.prototype, "animate", {configurable:true,value:animate});
  return animate;
}
function content(value: string, streaming = true) {
  return <StreamReveal value={value} streaming={streaming} durationMs={2000}>{enabled =>
    <p>{Array.from(value).map((text,index) => <span key={index} className={enabled ? "stream-reveal-glyph" : ""}
      data-reveal-start={index} data-reveal-end={index+1}>{text}</span>)}</p>
  }</StreamReveal>;
}
it("shows received text immediately on conversation mount/remount, revealing only new appends", async () => {
  const animate = animation();
  const host = document.createElement("div"); document.body.append(host);
  let root = createRoot(host);
  try {
    await act(async () => root.render(<StrictMode>{content("已收到正文")}</StrictMode>));
    expect([...host.querySelectorAll<HTMLElement>(".stream-reveal-glyph")].every(node => node.style.getPropertyValue("--stream-reveal-progress") === "1")).toBe(true);
    expect(animate).not.toHaveBeenCalled();
    const before = animate.mock.calls.length;
    await act(async () => root.render(<StrictMode>{content("已收到正文新")}</StrictMode>));
    const glyphs = [...host.querySelectorAll<HTMLElement>(".stream-reveal-glyph")];
    expect(glyphs.slice(0,-1).every(node => node.style.getPropertyValue("--stream-reveal-progress") === "1")).toBe(true);
    expect(Number(glyphs.at(-1)?.style.getPropertyValue("--stream-reveal-progress"))).toBeLessThan(1);
    expect(animate).toHaveBeenCalledTimes(before);
    await act(async () => root.unmount()); root = createRoot(host);
    await act(async () => root.render(content("已收到正文新")));
    expect([...host.querySelectorAll<HTMLElement>(".stream-reveal-glyph")].every(node => node.style.getPropertyValue("--stream-reveal-progress") === "1")).toBe(true);
    expect(animate).not.toHaveBeenCalled();
    await act(async () => root.render(content("已收到正文新追加")));
    const tail = host.querySelector<HTMLElement>(".stream-reveal-glyph:last-child")!;
    expect(Number(tail.style.getPropertyValue("--stream-reveal-progress"))).toBeLessThan(1);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
it("settles background text on focus/visibility restoration and releases listeners when detached", async () => {
  const animate = animation();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(content("旧")));
    await act(async () => root.render(content("旧后台收到的正文")));
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect([...host.querySelectorAll<HTMLElement>(".stream-reveal-glyph")].every(node => node.style.getPropertyValue("--stream-reveal-progress") === "1")).toBe(true);
    expect(animate).not.toHaveBeenCalled();
    await act(async () => root.render(content("旧后台收到的正文，又有新增")));
    vi.spyOn(document,"visibilityState","get").mockReturnValue("visible");
    await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    expect([...host.querySelectorAll<HTMLElement>(".stream-reveal-glyph")].every(node => node.style.getPropertyValue("--stream-reveal-progress") === "1")).toBe(true);
    await act(async () => root.unmount());
    const before = animate.mock.calls.length;
    window.dispatchEvent(new Event("focus")); document.dispatchEvent(new Event("visibilitychange"));
    expect(animate).toHaveBeenCalledTimes(before);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
it("keeps purely static history immediately visible without restoring animation", async () => {
  const animate = animation(); const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(content("已完成的历史消息", false)));
    window.dispatchEvent(new Event("focus"));
    expect(animate).not.toHaveBeenCalled();
    expect(host.textContent).toBe("已完成的历史消息");
  } finally { await act(async () => root.unmount()); host.remove(); }
});
it("respects reduced motion when restoring already received text", async () => {
  const animate = animation(); const original = window.matchMedia.bind(window);
  vi.spyOn(window,"matchMedia").mockImplementation(query => {
    const result = original(query); Object.defineProperty(result,"matches",{value:true}); return result;
  });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(content("已有内容")));
    expect(animate).not.toHaveBeenCalled();
    expect([...host.querySelectorAll<HTMLElement>(".stream-reveal-glyph")].every(node => node.style.getPropertyValue("--stream-reveal-progress") === "1")).toBe(true);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
