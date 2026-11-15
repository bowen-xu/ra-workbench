// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { StructuredReviewControls } from "./StructuredReviewControls";
import { EditorDiffControls } from "../editor-diff/EditorDiffControls";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let dispose: (() => Promise<void>) | undefined;
afterEach(async () => { await dispose?.(); dispose = undefined; document.body.innerHTML = ""; });

it("moves bulk review into concise menu actions with detailed focus hints", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); dispose = () => act(async () => root.unmount());
  const resolve = vi.fn(), keepUnreviewed = vi.fn();
  await act(async () => root.render(<StructuredReviewControls count={3} active={1} busy={false}
    onMove={vi.fn()} onResolve={resolve} onKeepUnreviewed={keepUnreviewed}
    metadataActive={false}/>));
  expect(host.querySelector(".writing-diff-bulk-group")).toBeNull();
  expect(host.querySelector('[aria-label="撤销阅读操作"]')).toBeNull();
  expect(host.querySelector('[aria-label="重做阅读操作"]')).toBeNull();
  const open = async () => { await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="更多阅读审阅操作"]')!.click()); };
  const item = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(value => value.textContent === label)!;
  await open();
  expect([...document.querySelectorAll('[role="menuitem"]')].map(value => value.textContent))
    .toEqual(["Undo all", "Keep all", "Keep 未校验"]);
  await act(async () => {
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    item("Undo all").focus();
  });
  await vi.waitFor(() => expect([...document.querySelectorAll('[role="tooltip"]:not([hidden])')]
    .some(value => value.textContent?.includes("本次已 Keep 的内容保留"))).toBe(true));
  await act(async () => item("Undo all").click()); expect(resolve).toHaveBeenLastCalledWith("undo", "all");
  await open(); await act(async () => item("Keep all").click()); expect(resolve).toHaveBeenLastCalledWith("keep", "all");
  await open(); await act(async () => item("Keep 未校验").click()); expect(keepUnreviewed).toHaveBeenCalledOnce();
});

it("preserves bulk controls for writing hosts by default", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); dispose = () => act(async () => root.unmount());
  await act(async () => root.render(<EditorDiffControls count={2} active={1} onMove={vi.fn()} onResolve={vi.fn()}/>));
  expect(host.querySelector(".writing-diff-bulk-group")?.textContent).toBe("Undo allKeep all");
});
