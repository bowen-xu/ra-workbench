/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";

import { ShimmerText } from "./ShimmerText";

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("renders one copy of the text and preserves host styling and accessibility semantics", async () => {
  await act(async () => root.render(<ShimmerText className="host-label" aria-hidden="true">准备中……</ShimmerText>));
  const text = host.querySelector("span")!;
  expect(host.textContent).toBe("准备中……");
  expect(host.querySelectorAll("span")).toHaveLength(1);
  expect(text.classList.contains("host-label")).toBe(true);
  expect(text.getAttribute("aria-hidden")).toBe("true");
  expect(text.hasAttribute("role")).toBe(false);
  expect(text.hasAttribute("aria-live")).toBe(false);
});

it("switches to static text without remounting and lets the host announce the new state", async () => {
  await act(async () => root.render(<ShimmerText role="status" aria-live="polite">加载中……</ShimmerText>));
  const text = host.querySelector("span")!;
  expect(text.getAttribute("data-shimmer-active")).toBe("true");
  await act(async () => root.render(<ShimmerText active={false} role="status" aria-live="polite">已就绪</ShimmerText>));
  expect(host.querySelector("span")).toBe(text);
  expect(text.textContent).toBe("已就绪");
  expect(text.getAttribute("data-shimmer-active")).toBe("false");
  expect(text.getAttribute("role")).toBe("status");
  expect(text.getAttribute("aria-live")).toBe("polite");
});

it("defaults to a 50% band and a 0.5-second sweep followed by a separate 3-second pause", async () => {
  await act(async () => root.render(<ShimmerText>准备中……</ShimmerText>));
  const style = host.querySelector("span")!.style;
  expect(style.getPropertyValue("--shimmer-mask-size")).toBe("250%");
  expect(style.getPropertyValue("--shimmer-band-start")).toBe("40%");
  expect(style.getPropertyValue("--shimmer-band-end")).toBe("60%");
  expect(style.getPropertyValue("--shimmer-center-opacity")).toBe("0.15");
  expect(style.getPropertyValue("--shimmer-cycle-duration")).toBe("3.5s");
  const sweepPortion = Number(style.getPropertyValue("--shimmer-timing").match(/1 ([\d.]+)%/)?.[1]) / 100;
  expect(sweepPortion * 3.5).toBeCloseTo(0.5);
  expect((1 - sweepPortion) * 3.5).toBeCloseTo(3);
});

it.each([25, 50, 100, 200])("keeps a %s%% band entirely outside the text at both ends without changing sweep time", async (width) => {
  await act(async () => root.render(<ShimmerText maskWidthPercent={width} sweepDurationSeconds={1} intervalSeconds={2}>参考文字</ShimmerText>));
  const style = host.querySelector("span")!.style;
  const canvas = Number.parseFloat(style.getPropertyValue("--shimmer-mask-size")) / 100;
  const start = Number.parseFloat(style.getPropertyValue("--shimmer-band-start")) / 100;
  const end = Number.parseFloat(style.getPropertyValue("--shimmer-band-end")) / 100;
  const band = (end - start) * canvas;
  expect(band).toBeCloseTo(width / 100);
  // Normalize text width to 1. Percentage mask-position aligns the corresponding
  // image edge with the text edge; the faded band must just clear the text.
  expect(1 - canvas / 2 + band / 2).toBeCloseTo(0);
  expect(canvas / 2 - band / 2).toBeCloseTo(1);
  expect(style.getPropertyValue("--shimmer-cycle-duration")).toBe("3s");
  const sweepPortion = Number(style.getPropertyValue("--shimmer-timing").match(/1 ([\d.]+)%/)?.[1]) / 100;
  expect(sweepPortion * 3).toBeCloseTo(1);
});

it("supports continuous sweeps and a zero-width static band, and keeps caller styles", async () => {
  await act(async () => root.render(<ShimmerText intervalSeconds={0} centerOpacityPercent={0} style={{ color: "red" }}>等待中</ShimmerText>));
  const text = host.querySelector("span")!;
  expect(text.style.getPropertyValue("--shimmer-timing")).toBe("linear");
  expect(text.style.getPropertyValue("--shimmer-cycle-duration")).toBe("0.5s");
  expect(text.style.getPropertyValue("--shimmer-center-opacity")).toBe("0");
  expect(text.style.color).toBe("red");
  await act(async () => root.render(<ShimmerText maskWidthPercent={0}>等待中</ShimmerText>));
  expect(text.getAttribute("data-shimmer-active")).toBe("false");
});

it("normalizes invalid parameters without producing invalid CSS", async () => {
  await act(async () => root.render(<ShimmerText maskWidthPercent={NaN} centerOpacityPercent={150}
    sweepDurationSeconds={-1} intervalSeconds={Infinity}>等待中</ShimmerText>));
  const style = host.querySelector("span")!.style;
  expect(style.getPropertyValue("--shimmer-mask-size")).toBe("250%");
  expect(style.getPropertyValue("--shimmer-center-opacity")).toBe("1");
  expect(style.getPropertyValue("--shimmer-cycle-duration")).toBe("3.5s");
});
