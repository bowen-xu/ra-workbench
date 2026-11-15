/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { NotificationEffect } from "./NotificationEffect";
import { notify } from "../services/notifications";

vi.mock("../services/notifications", () => ({ notify: { warning: vi.fn(), error: vi.fn(), info: vi.fn(), dismiss: vi.fn() } }));

it("uses shared notifications without a banner and removes only the old scope on replacement or unmount", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"), root = createRoot(host);
  const render = async (id: string, message: string | undefined) => {
    await act(async () => root.render(<StrictMode><NotificationEffect id={id} message={message} /></StrictMode>));
  };
  try {
    await render("manuscript-a:delivery", "交付记录待确认");
    expect(notify.warning).toHaveBeenLastCalledWith("交付记录待确认", { id: "manuscript-a:delivery" });
    expect(notify.dismiss).not.toHaveBeenCalled();
    const calls = vi.mocked(notify.warning).mock.calls.length;
    await render("manuscript-a:delivery", "交付记录待确认");
    expect(notify.warning).toHaveBeenCalledTimes(calls);
    expect(host.childNodes).toHaveLength(0);
    await render("manuscript-b:delivery", "读取失败");
    expect(notify.dismiss).toHaveBeenLastCalledWith("manuscript-a:delivery");
    expect(notify.warning).toHaveBeenLastCalledWith("读取失败", { id: "manuscript-b:delivery" });
    await render("manuscript-b:delivery", undefined);
    expect(notify.dismiss).toHaveBeenLastCalledWith("manuscript-b:delivery");
    expect(notify.warning).toHaveBeenCalledTimes(calls + 1);
    await render("manuscript-c:delivery", "等待核对");
  } finally {
    await act(async () => root.unmount());
    expect(notify.dismiss).toHaveBeenLastCalledWith("manuscript-c:delivery");
    vi.unstubAllGlobals(); vi.clearAllMocks();
  }
});
