/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";

import { ShimmerTextDemo } from "./ShimmerTextDemo";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

it("previews configurable wave dots independently of shimmer and resets all wave parameters", async () => {
  await act(async () => root.render(<ShimmerTextDemo />));
  expect(host.querySelector("h1")?.textContent).toBe("动效预览");
  const dots = host.querySelector<HTMLElement>(".wave-dots")!;
  const controls = host.querySelector<HTMLElement>('[aria-label="圆点参数"]')!;
  const direction = controls.querySelector("select")!;
  expect(direction.value).toBe("up");
  const upwardTiming = dots.style.getPropertyValue("--wave-dots-timing");
  await act(async () => {
    direction.value = "down";
    direction.dispatchEvent(new Event("change", { bubbles: true }));
  });
  expect(dots.style.getPropertyValue("--wave-dots-timing")).not.toBe(upwardTiming);
  const fields = controls.querySelectorAll<HTMLInputElement>("input");
  const values = [2, 1.4, 3, 6, 5];
  for (let index = 0; index < fields.length; index++) await setField(fields[index]!, String(values[index]));
  expect(dots.style.getPropertyValue("--wave-dots-duration")).toBe("2s");
  expect(dots.style.getPropertyValue("--wave-dots-wave-duration")).toBe("1.4s");
  await setField(fields[0]!, "0.5");
  expect(fields[1]!.value).toBe("0.5");
  expect(fields[1]!.max).toBe("0.5");
  await setField(fields[1]!, "1");
  expect(fields[1]!.value).toBe("0.5");
  expect(dots.style.getPropertyValue("--wave-dots-wave-duration")).toBe("0.5s");
  expect(dots.style.getPropertyValue("--wave-dots-amplitude")).toBe("3px");
  expect(dots.style.getPropertyValue("--wave-dots-size")).toBe("6px");
  expect(dots.style.getPropertyValue("--wave-dots-gap")).toBe("5px");
  const shimmer = host.querySelector<HTMLElement>(".shimmer-text")!;
  await act(async () => [...controls.querySelectorAll("button")].find(button => button.textContent === "停止动画")?.click());
  expect(dots.getAttribute("data-wave-dots-active")).toBe("false");
  expect(shimmer.getAttribute("data-shimmer-active")).toBe("true");
  await act(async () => [...controls.querySelectorAll("button")].find(button => button.textContent === "播放动画")?.click());
  expect(dots.getAttribute("data-wave-dots-active")).toBe("true");
  await act(async () => [...controls.querySelectorAll("button")].find(button => button.textContent === "恢复默认")?.click());
  expect([...fields].map(field => Number(field.value))).toEqual([1, 0.7, 2, 4, 3]);
  expect(direction.value).toBe("up");
  expect(dots.style.getPropertyValue("--wave-dots-duration")).toBe("1s");
  expect(dots.style.getPropertyValue("--wave-dots-wave-duration")).toBe("0.7s");
  expect(dots.getAttribute("data-wave-dots-active")).toBe("true");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

async function setField(field: HTMLInputElement | HTMLTextAreaElement, value: string): Promise<void> {
  await act(async () => {
    const prototype = field.tagName === "INPUT" ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(field, value);
    field.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it("updates the production shimmer from all preview controls and restores its defaults", async () => {
  await act(async () => root.render(<ShimmerTextDemo />));
  const preview = host.querySelector<HTMLElement>('[aria-label="效果预览"] .shimmer-text')!;
  expect(preview.textContent).toBe("准备中……");
  expect(preview.style.getPropertyValue("--shimmer-mask-size")).toBe("250%");
  await setField(host.querySelector("textarea")!, "正在加载资源……");
  const values = [80, 40, 1.25, 2.5];
  const fields = host.querySelectorAll<HTMLInputElement>('[aria-label="扫光参数"] input[type="number"]');
  for (let index = 0; index < fields.length; index++) await setField(fields[index]!, String(values[index]));
  expect(preview.textContent).toBe("正在加载资源……");
  expect(preview.style.getPropertyValue("--shimmer-mask-size")).toBe("280%");
  expect(preview.style.getPropertyValue("--shimmer-center-opacity")).toBe("0.4");
  expect(preview.style.getPropertyValue("--shimmer-cycle-duration")).toBe("3.75s");
  const label = host.querySelector<HTMLElement>(".shimmer-text-demo-label")!;
  const labelShimmer = label.parentElement!;
  expect(labelShimmer.classList.contains("shimmer")).toBe(true);
  expect(label.querySelector("svg")).not.toBeNull();
  expect(label.textContent).toBe("正在加载资源……");
  expect(labelShimmer.style.getPropertyValue("--shimmer-mask-size")).toBe("280%");
  expect(labelShimmer.style.getPropertyValue("--shimmer-center-opacity")).toBe("0.4");
  expect(labelShimmer.style.getPropertyValue("--shimmer-cycle-duration")).toBe("3.75s");
  const buttons = () => [...host.querySelectorAll<HTMLButtonElement>('[aria-label="扫光参数"] button')];
  await act(async () => buttons().find(button => button.textContent === "停止动画")?.click());
  expect(preview.getAttribute("data-shimmer-active")).toBe("false");
  expect(labelShimmer.getAttribute("data-shimmer-active")).toBe("false");
  await act(async () => buttons().find(button => button.textContent === "恢复默认")?.click());
  expect(preview.getAttribute("data-shimmer-active")).toBe("true");
  expect(preview.textContent).toBe("准备中……");
  expect(preview.style.getPropertyValue("--shimmer-mask-size")).toBe("250%");
  expect(preview.style.getPropertyValue("--shimmer-cycle-duration")).toBe("3.5s");
  expect(labelShimmer.getAttribute("data-shimmer-active")).toBe("true");
  expect(label.textContent).toBe("准备中……");
});
