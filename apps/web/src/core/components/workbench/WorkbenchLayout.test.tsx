// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {WorkbenchLayout} from "../../public/workbench";
import {WritingAgentPanel} from "../../features/agent/presentation/WritingAgentPanel";
import type {AgentModelGateway} from "../../features/agent/AgentModelControl";

// Evaluating any built-in declaration would violate standalone Core assembly.
vi.mock("../../../plugins/literature_library/public/plugin",()=>{throw new Error("literature plugin evaluated");});
vi.mock("../../../plugins/note_manager/public/plugin",()=>{throw new Error("note plugin evaluated");});
vi.mock("../../../plugins/writing_projects/public/plugin",()=>{throw new Error("writing plugin evaluated");});

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
let root:Root, host:HTMLDivElement;
beforeEach(()=>{host=document.createElement("div");document.body.append(host);root=createRoot(host);});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});

it("mounts the production workbench frame with no domain contributions",async()=>{
  await act(async()=>root.render(<WorkbenchLayout sidebarExpanded writingFocused={false}/>));
  expect(host.querySelector(".app-window > .app-shell > aside.app-sidebar")?.id).toBe("app-sidebar");
  expect(host.querySelector("aside")?.getAttribute("aria-label")).toBe("主导航");
  expect(host.querySelector(".brand")?.textContent).toBe("ResearchAssistant");
  expect(host.querySelector("nav")?.children.length).toBe(0);
  expect(host.querySelector(".app-shell")?.getAttribute("data-sidebar-expanded")).toBe("true");
});

it("retains slot identity and order when a declared host contributes its view",async()=>{
  const selected=vi.fn();
  const render=(expanded:boolean)=>root.render(<WorkbenchLayout sidebarExpanded={expanded} writingFocused={false}
    toolbar={<header data-testid="toolbar"/>} navigation={<button onClick={selected}>测试板块</button>}
    status={<div className="sidebar-note">已有存储</div>} utilityActions={<button>更新</button>}>
    <main className="main-area"><section data-testid="contributed-view">插件提供的内容</section></main>
  </WorkbenchLayout>);
  await act(async()=>render(true));
  const view=host.querySelector('[data-testid="contributed-view"]');
  expect(host.querySelector(".app-window")?.firstElementChild?.getAttribute("data-testid")).toBe("toolbar");
  expect(host.querySelector(".app-shell")?.children[1]?.tagName).toBe("MAIN");
  await act(async()=>host.querySelector<HTMLButtonElement>("nav button")!.click());
  expect(selected).toHaveBeenCalledOnce();
  await act(async()=>render(false));
  expect(host.querySelector('[data-testid="contributed-view"]')).toBe(view);
  expect(host.querySelector(".app-shell")?.getAttribute("data-sidebar-expanded")).toBe("false");
});

it("can host the real shared conversation panel with a missing domain disabled",async()=>{
  const submitted=vi.fn();
  const models:AgentModelGateway={
    getLlmConfiguration:async()=>({providers:[],defaultModels:[],useCases:[]}),
    fetchLlmProviderModels:async providerId=>({providerId,models:[]}),
  };
  await act(async()=>root.render(<WorkbenchLayout sidebarExpanded={false} writingFocused={false}>
    <main className="main-area"><WritingAgentPanel open independentContext showLauncher={false}
      modelGateway={models} historyReady inputDisabled disabledReason="当前工作台尚未接入写作对话"
      draft="保留的输入" onSubmit={submitted}/></main>
  </WorkbenchLayout>));
  expect(host.querySelector('[role="dialog"]')).not.toBeNull();
  expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("保留的输入");
  expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(true);
  expect(submitted).not.toHaveBeenCalled();
});
