// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentRuntimeCanvasPage } from "./AgentRuntimeCanvasPage";
import { getAppRuntime, selectAppRuntime } from "../../../core/runtime/appRuntime";
vi.mock("./NativeAgentCanvasPreview", () => ({ NativeAgentCanvasPreview: () => <div data-native-canvas /> }));
vi.mock("../../../core/components/AppToaster", () => ({ AppToaster: () => null }));

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("identifies the archived native route and keeps execution controls disabled", async () => {
  const previous = getAppRuntime();
  selectAppRuntime({ mode: "browser", storage: window.localStorage, gateways: {}, dispose() {} });
  const host = document.createElement("div"), root = createRoot(host);
  try {
    await act(async () => root.render(<AgentRuntimeCanvasPage />));
    expect(host.textContent).toContain("旧原生 HTTP 预览入口已归档");
    expect(host.textContent).toContain("pnpm dev:desktop");
    expect(host.querySelector("main.agent-canvas-demo")).not.toBeNull();
    expect(host.querySelector(".agent-canvas-demo-header h1")?.textContent).toBe("队列消息 UI 预览");
    const selector = host.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;
    expect(selector.disabled).toBe(true);
    expect(selector.options.length).toBeGreaterThan(0);
    expect(selector.querySelector('option[value="internalRunning"]')?.textContent).toContain("内部对话运行中");
    expect(selector.querySelector('option[value="resources"]')?.textContent).toContain("技能、工具和脚本");
    const reset = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "重置假数据");
    expect(reset?.disabled).toBe(true);
    expect(host.querySelector("[data-native-canvas]")).toBeNull();
    expect(host.querySelector("textarea,[contenteditable]" )).toBeNull();
  } finally { await act(async () => root.unmount()); selectAppRuntime(previous); }
});
