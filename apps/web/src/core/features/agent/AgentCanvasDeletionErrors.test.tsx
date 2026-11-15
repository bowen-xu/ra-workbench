// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentCanvasDeletionErrors } from "./AgentCanvasDeletionErrors";
import { notify } from "../../services/notifications";
vi.mock("../../services/notifications", () => ({notify: {error: vi.fn()}}));
let root: Root, host: HTMLDivElement;
beforeEach(() => {vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); host = document.createElement("div"); document.body.append(host); root = createRoot(host);});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); vi.clearAllMocks();});
function button(text: string) {return [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === text)!;}
it("only configures the scoped I/O port and guards duplicate arming until its ACK", async () => {
  let release!: () => void;
  const setNextDeletionFailure = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => {release = resolve;})).mockResolvedValue(undefined);
  await act(async () => root.render(<AgentCanvasDeletionErrors ready available faults={{setNextDeletionFailure}} />));
  await act(async () => {button("设置删除故障").click(); button("设置删除故障").click();});
  expect(setNextDeletionFailure).toHaveBeenCalledExactlyOnceWith("afterCommit"); expect(button("清除删除故障").disabled).toBe(true);
  expect(host.querySelector("dialog")).toBeNull(); await act(async () => release());
  expect(host.querySelector('[role="status"]')?.textContent).toContain("成功后丢回执仅一次");
  await act(async () => button("清除删除故障").click()); expect(setNextDeletionFailure).toHaveBeenLastCalledWith(null);
});
it("keeps unsupported backends and non-archived targets disabled", async () => {
  const setNextDeletionFailure = vi.fn();
  await act(async () => root.render(<AgentCanvasDeletionErrors ready available={false} faults={{setNextDeletionFailure}} />));
  expect(button("设置删除故障").disabled).toBe(true); expect(host.textContent).toContain("请重启原生预览");
  await act(async () => root.render(<AgentCanvasDeletionErrors ready={false} available faults={{setNextDeletionFailure}} />));
  expect(button("清除删除故障").disabled).toBe(true); expect(setNextDeletionFailure).not.toHaveBeenCalled();
});
it("reports the original safe fault configuration error without private bodies", async () => {
  const requestId = "00000000-0000-4000-8000-000000000001";
  const setNextDeletionFailure = vi.fn().mockRejectedValue({code: "AGENT_PREVIEW_GENERATION_CHANGED", status: 409, requestId, body: "private response"});
  await act(async () => root.render(<AgentCanvasDeletionErrors ready available faults={{setNextDeletionFailure}} />));
  await act(async () => button("设置删除故障").click());
  const message = vi.mocked(notify.error).mock.calls[0]![0];
  expect(message).toContain("AGENT_PREVIEW_GENERATION_CHANGED"); expect(message).toContain("HTTP 409"); expect(message).toContain(requestId);
  expect(message).not.toContain("private"); expect(button("设置删除故障").disabled).toBe(false);
});
