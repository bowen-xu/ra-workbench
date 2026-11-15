/** @vitest-environment happy-dom */
import {act, useRef} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {useReturnToBottomVisibility} from "./useReturnToBottomVisibility";
import {ReturnToBottomButton} from "./ReturnToBottomButton";

let resized: () => void;
function Fixture({open = true, running = false}: {readonly open?: boolean; readonly running?: boolean}) {
  const viewport = useRef<HTMLDivElement>(null), content = useRef<HTMLDivElement>(null);
  const {visible, hide} = useReturnToBottomVisibility(viewport, content, open);
  return <div ref={viewport}><div ref={content}/><ReturnToBottomButton visible={visible} running={running} onClick={hide}/></div>;
}
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) {resized = callback;}
    observe() {} disconnect() {}
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();});
const button = () => host.querySelector<HTMLButtonElement>(".writing-agent-scroll-bottom")!;
async function distance(px: number) {
  const viewport = host.firstElementChild as HTMLDivElement;
  Object.defineProperties(viewport, {scrollHeight: {configurable: true, value: 1000}, clientHeight: {configurable: true, value: 400}});
  viewport.scrollTop = 600 - px;
  await act(async () => viewport.dispatchEvent(new Event("scroll")));
}
it("uses two thresholds and retains state in both directions through the dead band", async () => {
  await act(async () => root.render(<Fixture/>));
  expect(button().disabled).toBe(true);
  for (const px of [0, 25, 27, 29]) {await distance(px); expect(button().disabled).toBe(true);}
  await distance(30); expect(button().disabled).toBe(false);
  for (const px of [29, 27, 26]) {await distance(px); expect(button().disabled).toBe(false);}
  await distance(25); expect(button().disabled).toBe(true);
  await distance(27); expect(button().disabled).toBe(true);
  await distance(-10); expect(button().disabled).toBe(true);
});
it("keeps the node mounted for fading, disables hidden interaction, and preserves running feedback", async () => {
  await act(async () => root.render(<Fixture running/>));
  const original = button();
  await distance(200);
  expect(button()).toBe(original);
  expect(original.closest("[data-visible]")?.getAttribute("aria-hidden")).toBe("false");
  expect(original.querySelectorAll(".writing-agent-scroll-bottom-dots > span")).toHaveLength(3);
  await act(async () => original.click());
  expect(button()).toBe(original); expect(original.disabled).toBe(true);
  expect(original.closest("[data-visible]")?.getAttribute("aria-hidden")).toBe("true");
  expect(original.closest("[data-visible]")?.hasAttribute("inert")).toBe(true);
});
it("remeasures content and viewport resize without moving the reading position, and releases on close", async () => {
  await act(async () => root.render(<Fixture/>)); await distance(200);
  const viewport = host.firstElementChild as HTMLDivElement;
  viewport.scrollTop = 580;
  await act(async () => resized());
  expect(button().disabled).toBe(true); expect(viewport.scrollTop).toBe(580);
  viewport.scrollTop = 400;
  await act(async () => resized()); expect(button().disabled).toBe(false);
  await act(async () => root.render(<Fixture open={false}/>)); expect(button().disabled).toBe(true);
  await act(async () => root.render(<Fixture/>)); expect(button().disabled).toBe(false);
});
