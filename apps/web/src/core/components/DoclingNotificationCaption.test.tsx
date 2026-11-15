// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { DoclingNotificationCaption } from "./DoclingNotificationCaption";

it("truncates paper titles at the end and filenames in the middle, and restores full short names", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const resizes: (() => void)[] = [];
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: (entries: ResizeObserverEntry[]) => void) { resizes.push(() => callback([])); }
    observe() {} disconnect() {}
  });
  let width = 160;
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(() => width);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => new DOMRect(0, 0, width, width ? 24 : 0));
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    font: "", measureText: (text: string) => ({ width: Array.from(text).length * 8 }),
  } as CanvasRenderingContext2D);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const paperTitle = "这是一篇很长的合成文献标题，用于测试末尾截断行为";
  const fileName = "Synthetic Research Document With Long Filename.pdf";
  try {
    await act(async () => root.render(<DoclingNotificationCaption paperTitle={paperTitle} fileName={fileName} />));
    const lines = host.querySelectorAll(".docling-notification-caption-line");
    expect(lines[0]?.textContent).toMatch(/^这是一篇.+\.\.\.$/);
    expect(lines[1]?.textContent).toMatch(/^Synthetic.*\.\.\..+\.pdf$/);
    expect(lines[0]?.getAttribute("aria-label")).toBe(paperTitle);
    expect(lines[1]?.getAttribute("aria-label")).toBe(fileName);
    await act(async () => {
      lines[1]?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
      lines[1]?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    });
    await vi.waitFor(() => expect(document.querySelector('[role="tooltip"]')?.textContent).toBe(fileName));
    const retained = [...lines].map(line => line.textContent);
    width = 0;
    await act(async () => resizes.forEach(resize => resize()));
    expect([...lines].map(line => line.textContent)).toEqual(retained);
    width = 800;
    await act(async () => resizes.forEach(resize => resize()));
    expect(lines[0]?.textContent).toBe(paperTitle);
    expect(lines[1]?.textContent).toBe(fileName);
    await act(async () => root.render(<DoclingNotificationCaption paperTitle="短标题" fileName="a.pdf" />));
    expect(host.textContent).toBe("短标题a.pdf");
  } finally {
    await act(async () => root.unmount());
    host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
  }
});
