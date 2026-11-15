/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { Shimmer } from "./Shimmer";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("masks one whole label subtree and preserves its semantics and styling", async () => {
  await act(async () => root.render(<Shimmer className="loading-label" maskWidthPercent={100}
    centerOpacityPercent={15} sweepDurationSeconds={1.5} intervalSeconds={0.5}>
    <label htmlFor="name" style={{ background: "green", border: "1px solid green" }}>
      <svg aria-hidden="true"><circle cx={5} cy={5} r={4} /></svg><span>姓名</span>
    </label>
  </Shimmer>));
  const wrapper = host.querySelector<HTMLElement>(".shimmer")!;
  expect(wrapper.parentElement).toBe(host);
  expect(wrapper.querySelectorAll("label")).toHaveLength(1);
  expect(wrapper.querySelector("label")?.htmlFor).toBe("name");
  expect(wrapper.querySelector("label")?.style.background).toBe("green");
  expect(wrapper.querySelector("svg")).not.toBeNull();
  expect(wrapper.classList.contains("loading-label")).toBe(true);
  expect(wrapper.style.getPropertyValue("--shimmer-mask-size")).toBe("300%");
  expect(wrapper.style.getPropertyValue("--shimmer-cycle-duration")).toBe("2s");
  expect(wrapper.style.getPropertyValue("--shimmer-timing")).toBe("linear(0, 1 75%, 1)");
  expect(wrapper.hasAttribute("role")).toBe(false);
  expect(wrapper.hasAttribute("aria-hidden")).toBe(false);
});

it("keeps controls mounted, focused and interactive when animation is stopped", async () => {
  const onClick = vi.fn();
  const render = (active: boolean) => <Shimmer active={active}>
    <input aria-label="草稿" defaultValue="初稿" /><button type="button" onClick={onClick}>保存</button>
  </Shimmer>;
  await act(async () => root.render(render(true)));
  const input = host.querySelector("input")!;
  input.value = "修改后的草稿";
  input.focus();
  await act(async () => root.render(render(false)));
  expect(host.querySelector("input")).toBe(input);
  expect(input.value).toBe("修改后的草稿");
  expect(document.activeElement).toBe(input);
  expect(host.querySelector(".shimmer")?.getAttribute("data-shimmer-active")).toBe("false");
  await act(async () => host.querySelector("button")!.click());
  expect(onClick).toHaveBeenCalledOnce();
});

it("supports a block container and forwards host attributes and styles", async () => {
  await act(async () => root.render(<Shimmer as="div" role="status" aria-live="polite" style={{ width: "100%" }}>
    <section><h2>正在加载资料</h2></section>
  </Shimmer>));
  const wrapper = host.querySelector<HTMLElement>(".shimmer")!;
  expect(wrapper.tagName).toBe("DIV");
  expect(wrapper.style.width).toBe("100%");
  expect(wrapper.getAttribute("role")).toBe("status");
  expect(wrapper.getAttribute("aria-live")).toBe("polite");
  expect(wrapper.querySelector("section")?.textContent).toBe("正在加载资料");
});
