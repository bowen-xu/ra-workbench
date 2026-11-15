/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {AgentCanvasPreviewFrame} from "./AgentCanvasPreviewFrame";
import {getHiddenNotifications, notify} from "../../../core/services/notifications";

vi.mock("../../../core/components/AppToaster", () => ({AppToaster: () => null}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

it("restores a hidden protected task from the shared production header control in the Canvas harness", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const id = "canvas-cancel-recovery-test", restore = vi.spyOn(notify, "showHidden");
  try {
    await act(async () => root.render(<AgentCanvasPreviewFrame controls={null}><div/></AgentCanvasPreviewFrame>));
    expect(host.querySelector(".app-notification-restore")).toBeNull();
    await act(async () => {notify.error("原取消尚待核对", {id, dismissible: false}); notify.hide(id);});
    expect(getHiddenNotifications().some(entry => entry.id === id)).toBe(true);
    const button = host.querySelector<HTMLButtonElement>('header button[aria-label="显示 1 条隐藏消息"]');
    expect(button).not.toBeNull();
    await act(async () => button!.click());
    expect(restore).toHaveBeenCalledOnce(); expect(getHiddenNotifications().some(entry => entry.id === id)).toBe(false);
    expect(host.querySelector(".app-notification-restore")).toBeNull();
  } finally {await act(async () => {notify.dismiss(id); root.unmount();}); host.remove(); vi.restoreAllMocks();}
});
