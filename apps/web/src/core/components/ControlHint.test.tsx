// @vitest-environment happy-dom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { ControlHint } from "./ControlHint";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
});

function visibleHint(): Element | null {
  return document.querySelector('[role="tooltip"]:not([hidden])');
}

it("keeps the anchor identity and handlers when an optional hint appears or disappears", async () => {
  const onClick = vi.fn();
  const render = (content: string | undefined) => root.render(<ControlHint content={content}><button onClick={onClick}>操作</button></ControlHint>);
  await act(async () => render(undefined));
  const button = container.querySelector("button")!;
  await act(async () => {
    button.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(350);
  });
  expect(visibleHint()).toBeNull();
  await act(async () => render("失败详情"));
  expect(container.querySelector("button")).toBe(button);
  await act(async () => {
    button.dispatchEvent(new MouseEvent("mouseout", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    button.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(350);
  });
  expect(visibleHint()?.textContent).toBe("失败详情");
  await act(async () => render(undefined));
  expect(visibleHint()).toBeNull();
  expect(container.querySelector("button")).toBe(button);
  await act(async () => button.click());
  expect(onClick).toHaveBeenCalledOnce();
});

it("does not render hidden content during parent updates and shows the latest hint", async () => {
  const renderContent = vi.fn();
  function Content({ label }: { readonly label: string }) { renderContent(); return <span>{label}</span>; }
  const hint = (label: string) => <ControlHint content={<Content label={label} />}><button>操作</button></ControlHint>;
  await act(async () => root.render(hint("旧提示")));
  for (let index = 0; index < 20; index++) await act(async () => root.render(hint("最新提示")));
  expect(renderContent).not.toHaveBeenCalled();
  await act(async () => {
    container.querySelector("button")?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    container.querySelector("button")?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(350);
  });
  expect(visibleHint()?.textContent).toBe("最新提示");
});

it.each([false, true])("hides immediately when leaving the anchor for the hint (wrap=%s)", async wrap => {
  const onMouseLeave = vi.fn();
  await act(async () => root.render(<ControlHint content="提示" wrap={wrap}><button onMouseLeave={onMouseLeave}>操作</button></ControlHint>));
  const anchor = container.querySelector(wrap ? ".control-hint-anchor" : "button");
  expect(anchor).not.toBeNull();
  await act(async () => {
    anchor?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    anchor?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(350);
  });
  const hint = visibleHint();
  expect(hint?.textContent).toBe("提示");
  await act(async () => {
    container.querySelector("button")?.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: hint }));
  });
  expect(visibleHint()).toBeNull();
  expect(onMouseLeave).toHaveBeenCalledOnce();
  await act(async () => {
    hint?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(350);
  });
  expect(visibleHint()).toBeNull();
});

it("cancels delayed appearance when leaving before the hint opens", async () => {
  await act(async () => root.render(<ControlHint content="提示"><button>操作</button></ControlHint>));
  const anchor = container.querySelector("button");
  await act(async () => {
    anchor?.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    anchor?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(100);
    anchor?.dispatchEvent(new MouseEvent("mouseleave"));
    anchor?.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body }));
    await vi.advanceTimersByTimeAsync(350);
  });
  expect(visibleHint()).toBeNull();
});

it("keeps keyboard hints until focus leaves the control", async () => {
  await act(async () => root.render(<><ControlHint content="提示"><button>操作</button></ControlHint><button>其他</button></>));
  const buttons = container.querySelectorAll("button");
  await act(async () => {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    buttons[0]?.focus();
    await vi.advanceTimersByTimeAsync(350);
  });
  expect(visibleHint()?.textContent).toBe("提示");
  await act(async () => buttons[0]?.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
  expect(visibleHint()?.textContent).toBe("提示");
  await act(async () => {
    buttons[1]?.focus();
    await vi.advanceTimersByTimeAsync(350);
  });
  expect(visibleHint()).toBeNull();
});
