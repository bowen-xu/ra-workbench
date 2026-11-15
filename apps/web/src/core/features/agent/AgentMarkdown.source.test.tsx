// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AgentMarkdown } from "./AgentMarkdown";
import { SOURCE_NAVIGATION_EVENT, type SourceNavigationRequest } from "./sourceNavigation";
const cleanups: (() => void)[] = [];
vi.mock("./sourceDirectory", () => ({ sourceDirectory: () => {
  const note = { value: { kind: "note", snapshot: { page: { title: "文献笔记" } } }, child: async () => note };
  return note;
} }));
afterEach(() => { cleanups.splice(0).forEach(cleanup => cleanup()); vi.useRealTimers(); vi.unstubAllGlobals(); });
it("routes a combined block range internally and leaves malformed links inert", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(() => { act(() => root.unmount()); host.remove(); });
  const navigate = vi.fn((event: Event) => { event.preventDefault(); (event as CustomEvent<SourceNavigationRequest>).detail.resolve(); });
  window.addEventListener(SOURCE_NAVIGATION_EVENT, navigate); cleanups.push(() => window.removeEventListener(SOURCE_NAVIGATION_EVENT, navigate));
  await act(async () => root.render(<AgentMarkdown value="[笔记](source://notes/note:#block{b}#line{2-1:2-3}) [错误](source://../paper) [网站](https://example.com)" />));
  const link = host.querySelector<HTMLAnchorElement>('a[href^="source:"]')!;
  expect(link.target).toBe("");
  await act(async () => link.click());
  expect(navigate).toHaveBeenCalledOnce();
  expect((navigate.mock.calls[0]![0] as CustomEvent<SourceNavigationRequest>).detail.link.locator).toEqual({kind: "block", blockId: "b", range: {startLine: 2, startColumn: 1, endLine: 2, endColumn: 3}});
  expect(host.querySelector('[aria-disabled="true"]')?.textContent).toBe("错误");
  expect(host.querySelector('a[href="https://example.com"]')?.getAttribute("target")).toBe("_blank");
});
it("shows a note name instead of IDs or a compound block range in its shared hint", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(() => { act(() => root.unmount()); host.remove(); });
  await act(async () => root.render(<AgentMarkdown value="[笔记](source://notes/note:#block%7Bb%7D%23line%7B2-1:2-3%7D)" />));
  const link = host.querySelector<HTMLAnchorElement>('a[href^="source:"]')!;
  await act(async () => { link.focus(); await vi.advanceTimersByTimeAsync(350); });
  const hint = document.querySelector('[role="tooltip"]');
  expect(hint?.textContent).toBe("block in 文献笔记");
  expect(hint?.classList.contains("control-hint")).toBe(true);
});
