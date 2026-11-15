/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {AppToaster} from "../components/AppToaster";
import {notify} from "./notifications";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

it("renders recovery actions as icon buttons and retains pending ownership", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host), id = "cancel-multi-action-test";
  const message = "原取消结果尚待核对 [C=11111111-1111-4111-8111-111111111111；R=22222222-2222-4222-8222-222222222222；I=33333333-3333-4333-8333-333333333333；operation=44444444-4444-4444-8444-444444444444]";
  let finish: (() => void) | undefined;
  const run = vi.fn(() => new Promise<void>(resolve => {finish = resolve;}));
  try {
    await act(async () => {root.render(<AppToaster/>);});
    await act(async () => {notify.error(message, {id, dismissible: false, actions: [
      {id: "inspect", label: "核对原取消", onClick: run}, {id: "retry", label: "刷新预览后重新取消", onClick: vi.fn()},
    ]});});
    await vi.waitFor(() => expect(document.querySelector('.app-notification-actions button[aria-label="核对原取消"]')).not.toBeNull());
    const actions = document.querySelector<HTMLElement>(".app-notification-actions")!, toast = actions.closest("[data-sonner-toast]")!;
    expect(toast.classList.contains("app-toast")).toBe(true);
    expect(toast.querySelector("[data-content]")?.textContent).toContain(message);
    expect(actions.parentElement).toBe(toast);
    const buttons = () => [...document.querySelectorAll<HTMLButtonElement>(".app-notification-actions button")];
    expect(buttons().every(button => button.classList.contains("app-toast-icon-action") && !!button.querySelector("svg"))).toBe(true);
    await act(async () => buttons()[0]!.click());
    await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect(buttons()).toHaveLength(2); expect(buttons().every(button => button.disabled)).toBe(true);});
    expect(buttons()[0]!.getAttribute("aria-busy")).toBe("true");
    await act(async () => buttons()[0]!.click()); expect(run).toHaveBeenCalledOnce();
    await act(async () => finish!());
    await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect(buttons()).toHaveLength(2); expect(buttons().every(button => !button.disabled)).toBe(true);});
  } finally {await act(async () => {finish?.(); notify.dismiss(id); root.unmount();}); host.remove();}
});

it("keeps all-icon task actions on the existing compact layout without the text modifier", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host), id = "compact-icon-task-test";
  try {
    await act(async () => {root.render(<AppToaster/>);});
    await act(async () => {notify.loading("解析任务", {id, actions: [
      {id: "stop", label: "终止解析", icon: <svg aria-hidden="true"/>, onClick: vi.fn()},
    ]});});
    await vi.waitFor(() => expect(document.querySelector('button[aria-label="终止解析"]')).not.toBeNull());
    const action = document.querySelector('button[aria-label="终止解析"]')!;
    expect(action.classList.contains("app-toast-icon-action")).toBe(true);
    expect(action.closest(".app-notification-actions")?.classList.contains("app-notification-actions-text")).toBe(false);
  } finally {await act(async () => {notify.dismiss(id); root.unmount();}); host.remove();}
});

it("uses icon buttons and accessible hints for legacy action and cancel options", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host), id = "legacy-icon-actions-test";
  const retry = vi.fn(), back = vi.fn();
  try {
    await act(async () => {root.render(<AppToaster/>);});
    await act(async () => {notify.error("操作尚待处理", {id, dismissible: false,
      action: {label: "重试", onClick: retry}, cancel: {label: "返回编辑", onClick: back}});});
    await vi.waitFor(() => expect(document.querySelector('button[aria-label="重试"]')).not.toBeNull());
    const anchor = document.querySelector('button[aria-label="重试"]')!.closest(".control-hint-anchor")!;
    await act(async () => {
      anchor.dispatchEvent(new MouseEvent("mouseover", {bubbles: true}));
      anchor.dispatchEvent(new MouseEvent("mousemove", {bubbles: true}));
    });
    await vi.waitFor(() => expect(document.querySelector('[role="tooltip"]')?.textContent).toBe("重试"));
    for (const label of ["重试", "返回编辑"]) {
      const button = document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
      expect(button.classList.contains("app-toast-icon-action")).toBe(true);
      expect(button.querySelector("svg")).not.toBeNull();
      expect(button.textContent).toBe("");
      await act(async () => button.click());
    }
    expect(retry).toHaveBeenCalledOnce(); expect(back).toHaveBeenCalledOnce();
  } finally {await act(async () => {notify.dismiss(id); root.unmount();}); host.remove();}
});
