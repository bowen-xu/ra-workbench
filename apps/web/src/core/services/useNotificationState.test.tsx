// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useNotificationState } from "./useNotificationState";
import { notify } from "./notifications";

vi.mock("./notifications", () => ({ notify: { error: vi.fn(), info: vi.fn() } }));
let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
let update: ReturnType<typeof useNotificationState>[1];
function RecoveryOwner({ kind }: { readonly kind: "error" | "info" }) {
  const [message, setMessage] = useNotificationState(kind);
  update = setMessage;
  return <button disabled={!!message}>编辑</button>;
}
beforeEach(() => {
  vi.clearAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("reports repeated failures while retaining the owner's recovery gate until explicit reconciliation", async () => {
  await act(async () => root.render(<RecoveryOwner kind="error" />));
  await act(async () => update("保存结果未确认"));
  await act(async () => update("保存结果未确认"));
  expect(notify.error).toHaveBeenCalledTimes(2);
  expect(host.querySelector("button")?.disabled).toBe(true);
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await act(async () => update(undefined));
  expect(notify.error).toHaveBeenCalledTimes(2);
  expect(host.querySelector("button")?.disabled).toBe(false);
});
it("routes information through the same tool and preserves ordered functional updates", async () => {
  await act(async () => root.render(<RecoveryOwner kind="info" />));
  await act(async () => { update("已恢复"); update(previous => `${previous}草稿`); });
  expect(notify.info).toHaveBeenLastCalledWith("已恢复草稿");
  expect(notify.error).not.toHaveBeenCalled();
});
