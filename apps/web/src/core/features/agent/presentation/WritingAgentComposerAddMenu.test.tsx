/** @vitest-environment happy-dom */
import { act as reactAct, StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { WritingAgentComposerAddMenu } from "./WritingAgentComposerAddMenu";
import type { WritingAgentContextScope } from "./WritingAgentContextControl";
import { createMockAgentResourceGateways } from "../../../../app/previews/agent/mockAgentResourceGateways";
import type { AgentResourceGateways } from "../agentResourceGateways";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => { document.body.replaceChildren(); });
async function act(action: () => unknown): Promise<void> {
  await reactAct(async () => { await action(); });
  await reactAct(async () => { await new Promise<void>((resolve) => requestAnimationFrame(() => resolve())); });
}
function Harness({ resources }: { resources?: AgentResourceGateways }) {
  const [mockResources] = useState(()=>{const resources=createMockAgentResourceGateways();void resources.tools.install(["模拟文献检索"]);return resources;});
  const [goalMode, setGoalMode] = useState(false);
  const [includeConceptRetrieval, setRetrieval] = useState(false);
  const [contextScope, setScope] = useState<WritingAgentContextScope>("current-file");
  return <WritingAgentComposerAddMenu activePath="main.tex" contextScope={contextScope} disabled={false} goalMode={goalMode}
    includeConceptRetrieval={includeConceptRetrieval} onContextScopeChange={setScope} onGoalModeChange={setGoalMode}
    onIncludeConceptRetrievalChange={setRetrieval} resourceGateways={resources ?? mockResources} />;
}
async function mount(resources?: AgentResourceGateways) {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  await act(() => root.render(<StrictMode><Harness {...(resources ? { resources } : {})} /></StrictMode>));
  const trigger = document.body.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')!;
  await act(() => { trigger.focus(); trigger.click(); });
  return { root, trigger };
}

it("completes a pointer selection after transient WebKit blur and closes without form controls", async () => {
  const { root, trigger } = await mount();
  try {
    expect(document.body.querySelector('input')).toBeNull();
    const goal = document.body.querySelector<HTMLButtonElement>('[aria-label="目标模式"]')!;
    await act(() => {
      goal.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      document.activeElement?.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null }));
      goal.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
      goal.click();
    });
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(trigger);
    await act(() => trigger.click());
    expect(document.body.querySelector('[aria-label="目标模式"]')?.textContent).toContain("已启用");
  } finally { await act(() => root.unmount()); }
});

it("navigates across flat groups without focusing their titles and restores trigger focus on Escape", async () => {
  const { root, trigger } = await mount();
  try {
    expect(document.activeElement?.getAttribute("aria-label")).toBe("文件");
    await act(() => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true })));
    expect(document.activeElement?.getAttribute("aria-label")).toBe("目标模式");
    await act(() => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true })));
    expect(document.activeElement?.getAttribute("aria-label")).toBe("概念网络检索");
    for (const selector of ['.agent-resource-group-tools button', '.agent-resource-group-skills button']) {
      await act(() => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true })));
      expect(document.activeElement?.matches(selector)).toBe(true);
    }
    await act(() => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(trigger);
  } finally { await act(() => root.unmount()); }
});

it("shows file search only after Tab expands the file entry", async () => {
  const {root, trigger} = await mount();
  try {
    expect(document.body.querySelector('input[aria-label="搜索文件"]')).toBeNull();
    const files = document.body.querySelector<HTMLButtonElement>('[aria-label="文件"]')!;
    expect(files.textContent).toContain("Tab 查看更多");
    await act(() => {files.focus(); files.dispatchEvent(new KeyboardEvent("keydown", {key: "Tab", bubbles: true, cancelable: true}));});
    const input = document.body.querySelector<HTMLInputElement>('input[aria-label="搜索文件"]');
    expect(document.activeElement).toBe(input);
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    await act(() => input!.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true})));
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    await act(() => trigger.click());
    expect(document.body.querySelector('input')).toBeNull();
  } finally {await act(() => root.unmount());}
});

it.each(["工具", "技能"])("lists production %s actions below a static title and closes the single menu on selection", async (kind) => {
  const { root, trigger } = await mount();
  try {
    expect([...document.body.querySelectorAll(".writing-agent-composer-add-section-title")].map(title => title.textContent)).toEqual(["工具", "技能"]);
    expect(document.body.textContent).not.toContain("上下文范围");
    expect(document.body.querySelector('[aria-label="当前文件上下文"]')).toBeNull();
    expect(document.body.querySelector('[aria-label="配置 Agent 工具"]')).toBeNull();
    expect(document.body.querySelector('[aria-label="配置 Agent 技能"]')).toBeNull();
    const menu = document.body.querySelector('[role="menu"]');
    expect(document.body.querySelectorAll('[role="menu"]')).toHaveLength(1);
    const item = document.body.querySelector<HTMLButtonElement>(kind === "工具" ? ".agent-resource-group-tools button" : ".agent-resource-group-skills button")!;
    expect(item.closest('[role="menu"]')).toBe(menu);
    expect(item.textContent).toContain(kind === "工具" ? "模拟文献检索" : "模拟写作技能");
    await act(() => {
      item.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      document.activeElement?.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null }));
      item.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
      item.click();
    });
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(trigger);
    await act(() => trigger.click());
    expect(document.body.querySelector(kind === "工具" ? ".agent-resource-group-tools button" : ".agent-resource-group-skills button")?.textContent).toContain(kind === "工具" ? "已授权服务器" : "应用技能");
  } finally { await act(() => root.unmount()); }
});

it("keeps static group titles and displays directory empty/error states without submenu controls", async () => {
  const resources = createMockAgentResourceGateways();
  resources.tools.list = vi.fn(async () => []);
  resources.skills.list = vi.fn(async () => { throw new Error("offline"); });
  const { root } = await mount(resources);
  try {
    expect(resources.tools.list).toHaveBeenCalledOnce();
    expect(resources.skills.list).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("暂无可用资源");
    expect(document.body.textContent).toContain("部分资源读取失败，请重新打开重试。");
    expect([...document.body.querySelectorAll(".writing-agent-composer-add-section-title")].every(title => !title.hasAttribute("tabindex") && title.getAttribute("role") !== "menuitem")).toBe(true);
  } finally { await act(() => root.unmount()); }
});
