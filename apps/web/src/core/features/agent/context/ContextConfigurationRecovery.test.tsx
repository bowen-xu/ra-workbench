// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ContextConfigurationRecovery } from "./ContextConfigurationRecovery";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it("shows both ordinary selection branches and forwards the explicit choice", async () => {
  const host = document.createElement("div"), root = createRoot(host), resolve = vi.fn();
  document.body.append(host);
  await act(async () => root.render(<ContextConfigurationRecovery conversationId="synthetic" recovery="conflict" onRetry={() => undefined} onRepair={() => undefined} onResolve={resolve}
    conflicts={[{ id: "conflict", path: ["persistent"], beforePath: ["persistent"], kind: "node", before: [],
      after: [{ label: "当前文献", resource: { kind: "paperMetadata", resourceId: "a" } }],
      incoming: [{ label: "其他文献", resource: { kind: "paperMetadata", resourceId: "b" } }] }]} />));
  expect(host.textContent).toContain("当前文献"); expect(host.textContent).toContain("其他文献");
  const button = [...host.querySelectorAll("button")].find(value => value.textContent === "采用其他更新");
  expect(button).toBeDefined(); await act(async () => button!.click());
  expect(resolve).toHaveBeenCalledExactlyOnceWith("conflict", "incoming");
  await act(async () => root.unmount()); host.remove();
});

it("uses the shared error presentation and retains diagnostics without exposing internal details", async () => {
  const host = document.createElement("div"), root = createRoot(host), retry = vi.fn(), repair = vi.fn();
  document.body.append(host);
  await act(async () => root.render(<ContextConfigurationRecovery conversationId="synthetic" recovery="save" detail="code=DATA_SQL_COMMIT_UNKNOWN"
    conflicts={[]} onRetry={retry} onRepair={repair} onResolve={() => undefined} />));
  expect(host.querySelector(".writing-agent-message.assistant.failure")).not.toBeNull();
  expect(host.textContent).not.toContain("DATA_SQL_COMMIT_UNKNOWN");
  expect(host.querySelector('[aria-label="复制诊断信息"]')).not.toBeNull();
  await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "核对并重试")!.click());
  expect(retry).toHaveBeenCalledOnce(); expect(repair).not.toHaveBeenCalled();
  await act(async () => root.render(<ContextConfigurationRecovery conversationId="synthetic" recovery="local"
    conflicts={[]} onRetry={retry} onRepair={repair} onResolve={() => undefined} />));
  await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "重试保存")!.click());
  expect(repair).toHaveBeenCalledOnce();
  await act(async () => root.unmount()); host.remove();
});
