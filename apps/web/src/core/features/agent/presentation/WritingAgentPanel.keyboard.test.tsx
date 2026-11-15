/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {WritingAgentPanel} from "./WritingAgentPanel";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
beforeEach(() => {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {await act(async () => root.unmount()); host.remove();});

it.each([false, true])("blocks Tab traversal from header and composer, shift=%s", async shiftKey => {
  await act(async () => root.render(<WritingAgentPanel open/>));
  for (const selector of ['[aria-label="拖动切换聊天布局"]', "textarea"]) {
    const control = host.querySelector<HTMLElement>(selector)!;
    const event = new KeyboardEvent("keydown", {key: "Tab", shiftKey, bubbles: true, cancelable: true});
    await act(async () => {control.focus(); control.dispatchEvent(event);});
    expect(event.defaultPrevented).toBe(true); expect(document.activeElement).toBe(control);
  }
});
it("preserves explicit Tab actions and leaves outside controls unaffected", async () => {
  const action = vi.fn();
  await act(async () => root.render(<WritingAgentPanel open headerControls={<button data-tab-action
    onKeyDown={event => {if (event.key === "Tab") {event.preventDefault(); action();}}}>操作</button>}/>));
  const inside = host.querySelector("[data-tab-action]")!;
  const tab = new KeyboardEvent("keydown", {key: "Tab", bubbles: true, cancelable: true});
  await act(async () => inside.dispatchEvent(tab));
  expect(action).toHaveBeenCalledOnce(); expect(tab.defaultPrevented).toBe(true);
  const outside = document.createElement("button"); document.body.append(outside);
  const outsideTab = new KeyboardEvent("keydown", {key: "Tab", bubbles: true, cancelable: true});
  outside.dispatchEvent(outsideTab); outside.remove();
  expect(outsideTab.defaultPrevented).toBe(false);
});

it.each([false, true])("does not dispatch or clear an ordinary draft on Tab, running=%s", async isRunning => {
  const onSubmit = vi.fn(), onQueue = vi.fn(), onSteer = vi.fn();
  await act(async () => root.render(<WritingAgentPanel open isRunning={isRunning} draft="普通消息"
    onSubmit={onSubmit} onQueue={onQueue} onSteer={onSteer}/>));
  const input = host.querySelector("textarea")!;
  input.focus();
  for (const shiftKey of [false, true]) {
    const event = new KeyboardEvent("keydown", {key: "Tab", shiftKey, bubbles: true, cancelable: true});
    await act(async () => input.dispatchEvent(event));
    expect(event.defaultPrevented).toBe(true); expect(document.activeElement).toBe(input);
  }
  expect(input.value).toBe("普通消息");
  expect(onSubmit).not.toHaveBeenCalled(); expect(onQueue).not.toHaveBeenCalled(); expect(onSteer).not.toHaveBeenCalled();
});
