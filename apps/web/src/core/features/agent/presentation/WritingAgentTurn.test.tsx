/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { WritingAgentTurn } from "./WritingAgentTurn";

let host: HTMLDivElement, root: Root;
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it.each([false, true])("shows a static terminal status only for an unsuccessful final-only reply (failed=%s)", async failed => {
  await act(async () => root.render(<WritingAgentTurn turnId="turn" running={false} aborted={false}
    failed={failed} hasProcess={false} thinking={false} startedAtMs={1_000} finishedAtMs={4_900}
    finalMessage={<p data-test-final>Partial answer</p>}>{null}</WritingAgentTurn>));
  expect(host.querySelector("details, summary, svg")).toBeNull();
  expect(host.querySelector("[data-test-final]")?.textContent).toBe("Partial answer");
  expect(host.querySelector('[data-shimmer-active="true"]')).toBeNull();
  if (failed) {
    const status = host.querySelector('[data-agent-activity="failed"]')!;
    expect(status.getAttribute("role")).toBe("status");
    expect(status.querySelector(".writing-agent-turn-status")?.textContent).toBe("未完成 · 3秒");
    await act(async () => status.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host.querySelector("details, summary")).toBeNull();
  } else expect(host.querySelector(".writing-agent-turn-status, .writing-agent-turn-header")).toBeNull();
});

it("keeps an unsuccessful process in its existing disclosure without adding a second status", async () => {
  await act(async () => root.render(<WritingAgentTurn turnId="turn" running={false} aborted={false}
    failed hasProcess thinking={false} startedAtMs={1_000} finishedAtMs={4_900}
    finalMessage={<p>Final partial answer</p>}><p>Earlier process</p></WritingAgentTurn>));
  const disclosure = host.querySelector<HTMLDetailsElement>(".writing-agent-turn-process")!;
  expect(disclosure.open).toBe(false);
  expect(disclosure.querySelector("summary")?.textContent).toBe("未完成 · 3秒");
  expect(host.querySelectorAll(".writing-agent-turn-status")).toHaveLength(1);
  expect(host.querySelector('[data-agent-activity="failed"]')).toBeNull();
  await act(async () => disclosure.querySelector("summary")!.click());
  expect(disclosure.open).toBe(true);
});

it.each(["process", "aborted"] as const)("retains the same response DOM when the turn becomes %s", async change => {
  const render = (changed: boolean) => root.render(<WritingAgentTurn turnId="turn" running={!changed}
    aborted={changed && change === "aborted"} hasProcess={changed && change === "process"} thinking={false}
    finalMessage={null}><p data-process-text>Already displayed response</p></WritingAgentTurn>);
  await act(async () => render(false));
  const content = host.querySelector("[data-process-text]");
  expect(content).not.toBeNull();
  await act(async () => render(true));
  expect(host.querySelector("[data-process-text]")).toBe(content);
  if (change === "aborted") {
    expect(host.querySelector('[data-agent-activity="aborted"]')?.textContent).toContain("已中止");
    expect(host.querySelector<HTMLDetailsElement>("details")?.open).toBe(true);
    expect(host.querySelector("summary")?.hidden).toBe(true);
  }
});

it("keeps a numeric zero child visible without exposing a process disclosure", async () => {
  await act(async () => root.render(<WritingAgentTurn turnId="turn" running={false} aborted={false}
    hasProcess={false} thinking={false} finalMessage={null}>{0}</WritingAgentTurn>));
  expect(host.querySelector(".writing-agent-turn-content")?.textContent).toBe("0");
  expect(host.querySelector<HTMLDetailsElement>("details")?.open).toBe(true);
  expect(host.querySelector("summary")?.hidden).toBe(true);
});
