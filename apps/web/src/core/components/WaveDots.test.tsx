/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";

import { WaveDots, type WaveDotsProps } from "./WaveDots";

it("can stop and restart decoration without replacing its dots or accessible owner", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    const render = (active: boolean) => <button aria-label="回到底部"><WaveDots active={active} className="activity" /></button>;
    await act(async () => root.render(render(true)));
    const dots = host.querySelector(".wave-dots")!;
    const children = [...dots.children];
    expect(children).toHaveLength(3);
    expect(dots.getAttribute("aria-hidden")).toBe("true");
    expect(dots.getAttribute("role")).toBeNull();
    expect(dots.classList.contains("activity")).toBe(true);
    expect(dots.getAttribute("data-wave-dots-active")).toBe("true");
    await act(async () => root.render(render(false)));
    expect(host.querySelector(".wave-dots")).toBe(dots);
    expect([...dots.children]).toEqual(children);
    expect(dots.getAttribute("data-wave-dots-active")).toBe("false");
    expect(host.querySelector("button")?.getAttribute("aria-label")).toBe("回到底部");
    await act(async () => root.render(render(true)));
    expect(dots.getAttribute("data-wave-dots-active")).toBe("true");
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("updates wave parameters in place and handles invalid values safely", async () => {
  const host = document.createElement("div");
  const root = createRoot(host);
  try {
    const render = (props: WaveDotsProps) => <WaveDots {...props} style={{ color: "red" }} />;
    await act(async () => root.render(render({})));
    const dots = host.querySelector<HTMLElement>(".wave-dots")!;
    const children = [...dots.children];
    await act(async () => root.render(render({ durationSeconds: 2, waveDurationSeconds: 1.4, amplitudePx: 3, dotSizePx: 6, gapPx: 5 })));
    expect(host.querySelector(".wave-dots")).toBe(dots);
    expect([...dots.children]).toEqual(children);
    expect(dots.style.getPropertyValue("--wave-dots-duration")).toBe("2s");
    expect(dots.style.getPropertyValue("--wave-dots-wave-duration")).toBe("1.4s");
    expect(dots.style.getPropertyValue("--wave-dots-amplitude")).toBe("3px");
    expect(dots.style.getPropertyValue("--wave-dots-size")).toBe("6px");
    expect(dots.style.getPropertyValue("--wave-dots-gap")).toBe("5px");
    expect(dots.style.color).toBe("red");
    const upTiming = dots.style.getPropertyValue("--wave-dots-timing");
    await act(async () => root.render(render({ durationSeconds: 2, waveDurationSeconds: 1.4, startDirection: "down", amplitudePx: 3, dotSizePx: 6, gapPx: 5 })));
    expect(dots.style.getPropertyValue("--wave-dots-timing")).not.toBe(upTiming);
    await act(async () => root.render(render({ durationSeconds: 0, waveDurationSeconds: NaN, amplitudePx: NaN, dotSizePx: -1, gapPx: Infinity })));
    expect(dots.style.getPropertyValue("--wave-dots-duration")).toBe("1s");
    expect(dots.style.getPropertyValue("--wave-dots-wave-duration")).toBe("0.7s");
    expect(dots.style.getPropertyValue("--wave-dots-amplitude")).toBe("2px");
    expect(dots.style.getPropertyValue("--wave-dots-size")).toBe("4px");
    expect(dots.style.getPropertyValue("--wave-dots-gap")).toBe("3px");
    await act(async () => root.render(render({ amplitudePx: -2, gapPx: -1, durationSeconds: 0.5, waveDurationSeconds: 1 })));
    expect(dots.style.getPropertyValue("--wave-dots-amplitude")).toBe("0px");
    expect(dots.style.getPropertyValue("--wave-dots-gap")).toBe("0px");
    expect(dots.style.getPropertyValue("--wave-dots-wave-duration")).toBe("0.5s");
    await act(async () => root.render(render({ waveDurationSeconds: -1 })));
    expect(dots.style.getPropertyValue("--wave-dots-wave-duration")).toBe("0s");
    expect(dots.getAttribute("data-wave-dots-active")).toBe("false");
  } finally {
    await act(async () => root.unmount());
  }
});
