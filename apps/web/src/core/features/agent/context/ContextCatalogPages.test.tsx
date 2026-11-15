/** @vitest-environment happy-dom */
import {act, useState} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {EMPTY_AGENT_CONTEXT, selectAgentContext, type AgentContextSelection} from "@research-assistant/core/agent";
import {ContextCatalogPages} from "./ContextCatalogPages";

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
const rows: readonly AgentContextSelection[] = Array.from({length: 19}, (_, index) => ({
  label: `笔记 ${index + 1}`, resource: {kind: "noteGroup", resourceId: `note-${index + 1}`, parts: [
    {label: "正文", selected: true, access: "readWrite", resource: {kind: "note", resourceId: `note-${index + 1}`}},
  ]},
}));
let host: HTMLDivElement, root: Root, now = 0;
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); host = document.createElement("div"); document.body.append(host); root = createRoot(host); now = 0; vi.spyOn(performance, "now").mockImplementation(() => now); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function Harness({disabled = false, values = rows}: {readonly disabled?: boolean; readonly values?: readonly AgentContextSelection[]}) {
  const [state, setState] = useState(EMPTY_AGENT_CONTEXT);
  return <ContextCatalogPages rows={values} state={state} disabled={disabled} onChange={(item, remove) => setState(value => remove
    ? {...value, persistent: value.persistent.filter(row => row.resource.resourceId !== item.resource.resourceId)}
    : selectAgentContext(value, item, "persistent"))}/>;
}
function page(): string | undefined { return host.querySelector<HTMLElement>("[data-page]")?.dataset.page; }
async function finishAnimation(): Promise<void> {
  await act(async () => host.querySelector(".context-catalog-outgoing")?.dispatchEvent(new Event("animationend", {bubbles: true})));
}
async function click(label: string): Promise<void> {
  await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!.click());
  await finishAnimation();
}
async function wheel(deltaY: number, momentum = false): Promise<WheelEvent> {
  const event = new WheelEvent("wheel", {deltaY, bubbles: true, cancelable: true});
  Object.defineProperty(event, "momentum", {value: momentum});
  now += 300;
  Object.defineProperty(event, "timeStamp", {value: now});
  await act(async () => host.querySelector(".context-catalog-viewport")!.dispatchEvent(event));
  await finishAnimation();
  return event;
}
it("replaces pages and keeps selected resources when returning to an earlier page", async () => {
  await act(async () => root.render(<Harness/>));
  await click("使用 笔记 1");
  expect(host.querySelector<HTMLInputElement>('[aria-label="使用 笔记 1"]')!.checked).toBe(true);
  now += 300; await click("下一页上下文");
  expect(page()).toBe("2");
  expect(host.querySelector('[aria-label="使用 笔记 1"]')).toBeNull();
  expect(host.querySelectorAll(".context-catalog-item")).toHaveLength(6);
  now += 300; await click("上一页上下文");
  expect(page()).toBe("1");
  expect(host.querySelector<HTMLInputElement>('[aria-label="使用 笔记 1"]')!.checked).toBe(true);
});
it("does not page on momentum but accepts a new active swipe immediately", async () => {
  await act(async () => root.render(<Harness/>));
  await wheel(60); expect(page()).toBe("2");
  await wheel(24, true); await wheel(12, true); expect(page()).toBe("2");
  await wheel(60); expect(page()).toBe("3");
  await wheel(16, true); expect(page()).toBe("3");
  expect(host.querySelectorAll(".context-catalog-item")).toHaveLength(7);
});
it("accepts an active reversal before the detector's gesture-end timeout", async () => {
  await act(async () => root.render(<Harness/>));
  const viewport = host.querySelector<HTMLElement>(".context-catalog-viewport")!;
  async function packet(deltaY: number) {
    now += 300;
    await act(async () => viewport.dispatchEvent(new WheelEvent("wheel", {deltaY, bubbles: true, cancelable: true})));
    await finishAnimation();
  }
  await packet(180); expect(page()).toBe("2");
  await packet(-180); expect(page()).toBe("1");
});
it("scrolls expanded content first and requires a new gesture at its boundary", async () => {
  await act(async () => root.render(<Harness/>));
  const viewport = host.querySelector<HTMLElement>(".context-catalog-viewport")!;
  Object.defineProperties(viewport, {clientHeight: {value: 200}, scrollHeight: {value: 600}});
  expect((await wheel(60)).defaultPrevented).toBe(false);
  viewport.scrollTop = 400;
  // The same active stream has already been consumed by ordinary scrolling.
  const event = new WheelEvent("wheel", {deltaY: 55, bubbles: true, cancelable: true});
  Object.defineProperties(event, {momentum: {value: false}, timeStamp: {value: now + 8}});
  await act(async () => viewport.dispatchEvent(event));
  expect(page()).toBe("1"); expect(event.defaultPrevented).toBe(true);
  await wheel(10, true); await wheel(60);
  expect(page()).toBe("2");
});
it("allows reading and page navigation while context editing is disabled", async () => {
  await act(async () => root.render(<Harness disabled/>));
  expect(host.querySelector<HTMLInputElement>('[aria-label="使用 笔记 1"]')!.disabled).toBe(true);
  await click("下一页上下文"); expect(page()).toBe("2");
});
it("supports keyboard page turns without moving focus into a removed row", async () => {
  await act(async () => root.render(<Harness/>));
  const viewport = host.querySelector<HTMLElement>(".context-catalog-viewport")!;
  viewport.focus();
  await act(async () => viewport.dispatchEvent(new KeyboardEvent("keydown", {key: "PageDown", bubbles: true, cancelable: true})));
  expect(page()).toBe("2"); expect(document.activeElement).toBe(viewport);
});
it("shows previous with an up arrow above and next with a down arrow below, only when available", async () => {
  await act(async () => root.render(<Harness/>));
  expect(host.querySelector("footer")).toBeNull();
  const edges = host.querySelectorAll(".context-catalog-page-edge");
  expect(edges[0]!.querySelector("button")).toBeNull();
  expect(edges[1]!.querySelector('[aria-label="下一页上下文"] .lucide-chevron-down')).not.toBeNull();
  await click("下一页上下文"); now += 300;
  expect(edges[0]!.querySelector('[aria-label="上一页上下文"] .lucide-chevron-up')).not.toBeNull();
  expect(edges[1]!.querySelector('[aria-label="下一页上下文"] .lucide-chevron-down')).not.toBeNull();
  await click("下一页上下文");
  expect(edges[0]!.querySelector('[aria-label="上一页上下文"] .lucide-chevron-up')).not.toBeNull();
  expect(edges[1]!.querySelector("button")).toBeNull();
});
it("fits page capacity to the available height and balances rows without changing selections", async () => {
  let resize: ResizeObserverCallback | undefined;
  const disconnect = vi.fn();
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: ResizeObserverCallback) { resize = callback; }
    observe() {} disconnect = disconnect;
  });
  await act(async () => root.render(<Harness/>));
  await click("使用 笔记 1");
  await act(async () => resize!([{contentRect: {height: 350}} as ResizeObserverEntry], {} as ResizeObserver));
  expect(host.querySelectorAll(".context-catalog-viewport .context-catalog-item")).toHaveLength(9);
  expect(host.querySelector<HTMLInputElement>('[aria-label="使用 笔记 1"]')!.checked).toBe(true);
  await click("下一页上下文");
  expect(host.querySelectorAll(".context-catalog-item")).toHaveLength(10);
  expect(host.querySelector('[aria-label="下一页上下文"]')).toBeNull();
  await act(async () => root.unmount());
  expect(disconnect).toHaveBeenCalledOnce();
  root = createRoot(host);
});
it("keeps the departing page inert during the slide and blocks repeated turns", async () => {
  await act(async () => root.render(<Harness/>));
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="下一页上下文"]')!.click());
  const outgoing = host.querySelector<HTMLElement>(".context-catalog-outgoing")!;
  expect(outgoing.hasAttribute("inert")).toBe(true);
  expect(outgoing.getAttribute("aria-hidden")).toBe("true");
  expect(outgoing.querySelector<HTMLInputElement>("input")!.disabled).toBe(true);
  now += 100; await click("下一页上下文");
  expect(page()).toBe("2");
  expect(host.querySelector(".context-catalog-outgoing")).toBeNull();
});
it("changes pages immediately without retaining an outgoing page with reduced motion", async () => {
  vi.spyOn(window, "matchMedia").mockImplementation(query => ({matches: query === "(prefers-reduced-motion: reduce)"}) as MediaQueryList);
  await act(async () => root.render(<Harness/>));
  await click("下一页上下文");
  await click("下一页上下文");
  expect(page()).toBe("3");
  expect(host.querySelector(".context-catalog-outgoing")).toBeNull();
  await click("上一页上下文");
  expect(page()).toBe("2");
});
it.each([{values: []}, {values: rows.slice(0, 1)}])("hides both arrows for an empty or single-page catalog", async ({values}) => {
  await act(async () => root.render(<Harness values={values}/>));
  expect(host.querySelectorAll(".context-catalog-page-edge button")).toHaveLength(0);
  if (values.length === 0) expect(host.textContent).toContain("没有匹配的上下文对象");
});
