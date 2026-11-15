/** @vitest-environment happy-dom */
import {act, type ReactNode} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {AgentResourceSettings} from "./AgentResourceSettings";
import {createMockAgentResourceGateways} from "../../../app/previews/agent/mockAgentResourceGateways";
import type {agentResourceManagementGateway} from "../../services/agentResourceManagementGateway";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT=true;
afterEach(()=>document.body.replaceChildren());
async function render(node:ReactNode) {
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 await act(async()=>root.render(node));return {host,root};
}
function gateway():typeof agentResourceManagementGateway {
 return {chooseSkillDirectory:vi.fn(async()=>"/fixture/skills"),executionHosts:vi.fn(async()=>[]),authorizeShell:vi.fn(async()=>undefined),
 scan:vi.fn(async()=>({importId:"import",resolvedCommit:null,candidates:[{sourceId:".agents/skills/new/SKILL.md",name:"New skill",revision:"2",existingRevision:null},{sourceId:".agents/skills/old/SKILL.md",name:"Existing skill",revision:"2",existingRevision:"1"}]})),
 install:vi.fn(async()=>undefined),cancel:vi.fn(async()=>undefined),remove:vi.fn(async()=>undefined),
 readMcp:vi.fn(async()=>"schema_version = 1\n"),saveMcp:vi.fn(async()=>undefined),credential:vi.fn(async()=>undefined)};
}
function button(host:HTMLElement,label:string){return [...host.querySelectorAll<HTMLButtonElement>("button")].find(item=>item.textContent===label||item.getAttribute("aria-label")===label)!;}
it("imports multiple full packages only after an existing version is explicitly selected for update",async()=>{
 const management=gateway(),resources=createMockAgentResourceGateways();const {host,root}=await render(<AgentResourceSettings gateway={management} skills={resources.skills} tools={resources.tools}/>);
 try {
  await act(async()=>button(host,"导入本地目录").click());expect(management.chooseSkillDirectory).toHaveBeenCalledOnce();expect(management.scan).toHaveBeenCalledWith({kind:"local",directory:"/fixture/skills"});
  const checks=[...host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]')];expect(checks.map(item=>item.checked)).toEqual([true,false,false]);
  await act(async()=>checks[1]!.click());expect(button(host,"安装所选技能").disabled).toBe(true);
  await act(async()=>checks[2]!.click());expect(button(host,"安装所选技能").disabled).toBe(false);
  await act(async()=>button(host,"安装所选技能").click());expect(management.install).toHaveBeenCalledWith("import",[{sourceId:".agents/skills/new/SKILL.md",expectedExistingRevision:null},{sourceId:".agents/skills/old/SKILL.md",expectedExistingRevision:"1"}]);
  expect(host.querySelector('[aria-label="选择导入技能"]')).toBeNull();
 }finally{await act(async()=>root.unmount());}
});
it("cancels directory selection through the injected gateway without scanning or changing the import preview",async()=>{
 const management=gateway(),resources=createMockAgentResourceGateways();management.chooseSkillDirectory=vi.fn(async()=>null);const {host,root}=await render(<AgentResourceSettings gateway={management} skills={resources.skills} tools={resources.tools}/>);
 try {
  await act(async()=>button(host,"导入本地目录").click());expect(management.chooseSkillDirectory).toHaveBeenCalledOnce();expect(management.scan).not.toHaveBeenCalled();expect(management.install).not.toHaveBeenCalled();
  expect(host.querySelector('[aria-label="选择导入技能"]')).toBeNull();expect(button(host,"导入本地目录").disabled).toBe(false);
  vi.mocked(management.chooseSkillDirectory).mockResolvedValueOnce("/fixture/skills");
  await act(async()=>button(host,"导入本地目录").click());const preview=host.querySelector('[aria-label="选择导入技能"]');expect(preview).not.toBeNull();
  await act(async()=>button(host,"导入本地目录").click());expect(management.scan).toHaveBeenCalledOnce();expect(management.cancel).not.toHaveBeenCalled();expect(management.install).not.toHaveBeenCalled();
  expect(host.querySelector('[aria-label="选择导入技能"]')).toBe(preview);expect(preview?.querySelector<HTMLInputElement>('input[type="checkbox"]')?.checked).toBe(true);
 }finally{await act(async()=>root.unmount());}
});
it("retains edited MCP configuration on a concurrent-change error and sends the original text as CAS",async()=>{
 const management=gateway(),resources=createMockAgentResourceGateways();management.saveMcp=vi.fn(async()=>{throw new Error("配置已变化，请重试");});const {host,root}=await render(<AgentResourceSettings gateway={management} skills={resources.skills} tools={resources.tools}/>);
 try {
  await act(async()=>button(host,"编辑 MCP 配置").click());
  const input=host.querySelector<HTMLTextAreaElement>('textarea[aria-label="MCP 服务器配置"]')!,updated='schema_version = 1\n# changed\n';
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,updated);input.dispatchEvent(new Event("input",{bubbles:true}));});
  await act(async()=>button(host,"保存服务器配置").click());expect(management.saveMcp).toHaveBeenCalledWith(updated,"schema_version = 1\n");expect(input.value).toBe(updated);expect(host.textContent).toContain("配置已变化，请重试");
 }finally{await act(async()=>root.unmount());}
});

it("adds an HTTP MCP from the structured form without implicitly authorizing it",async()=>{
 const management=gateway(),resources=createMockAgentResourceGateways();const {host,root}=await render(<AgentResourceSettings gateway={management} skills={resources.skills} tools={resources.tools}/>);
 const approve=vi.spyOn(resources.tools,"install");
 try {
  await act(async()=>button(host,"添加资源").click());
  const add=[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(item=>item.textContent==="添加 MCP 服务器")!;
  await act(async()=>add.click()); await act(async()=>button(host,"Streamable HTTP").click());
  for(const [label,value] of [["MCP 服务器名称","fixture"],["MCP 服务地址","https://example.com/mcp"]]){
   const field=host.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
   await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(field,value);field.dispatchEvent(new Event("input",{bubbles:true}));});
  }
  await act(async()=>host.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));
  expect(management.saveMcp).toHaveBeenCalledWith(expect.stringContaining('transport = "streamable_http"'),"schema_version = 1\n");
  expect(approve).not.toHaveBeenCalled();
 }finally{await act(async()=>root.unmount());}
});
