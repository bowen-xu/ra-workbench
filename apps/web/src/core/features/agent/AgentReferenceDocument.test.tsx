/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AgentReferenceDocument } from "./AgentReferenceDocument";
import { createMockAgentResourceGateways } from "../../../app/previews/agent/mockAgentResourceGateways";
import type { AgentReferenceInput } from "@research-assistant/core/agent";
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });
const skill: Extract<AgentReferenceInput, {type: "skill"}> = { type: "skill", name: "模拟写作技能", identity: { environment_id: "application", source_id: "mock/academic-writing/SKILL.md", revision: "1" } };
it("opens the exact bound document through the injected gateway", async () => {
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.removeAttribute("open"); });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const gateways = createMockAgentResourceGateways(); const read = vi.fn(gateways.readSkillDocument); const close = vi.fn();
  const {OPEN_TEXT_DOCUMENT}=await import("../../components/text-editor/textDocumentEvents");
  let opened: import("../../components/text-editor/textDocumentEvents").TextDocumentRequest | undefined;
  const listener=(event:Event)=>{event.preventDefault();opened=(event as CustomEvent<typeof opened>).detail;};
  window.addEventListener(OPEN_TEXT_DOCUMENT,listener);
  try {
    await act(async () => root.render(<AgentReferenceDocument reference={skill} onClose={close} gateways={{...gateways, readSkillDocument: read}} />));
    expect(read).toHaveBeenCalledWith(skill.identity, undefined);
    await act(async () => {await read.mock.results[0]!.value;});
    expect(opened?.content).toContain("整理摘要、核对论证并保留来源");
    expect(document.querySelector("dialog")).toBeNull();
    expect(close).toHaveBeenCalledOnce();
  } finally { window.removeEventListener(OPEN_TEXT_DOCUMENT,listener); await act(async () => root.unmount()); }
});

it("opens a file's current object without a revision or metadata dialog", async () => {
  const {SOURCE_NAVIGATION_EVENT} = await import("./sourceNavigation");
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host), close=vi.fn();
  let path: readonly string[] | undefined;
  const listener=(event: Event)=>{
    const request=event as CustomEvent<import("./sourceNavigation").SourceNavigationRequest>;
    event.preventDefault();path=request.detail.link.path;request.detail.resolve();
  };
  window.addEventListener(SOURCE_NAVIGATION_EVENT,listener);
  try {
    await act(async()=>root.render(<AgentReferenceDocument reference={{type:"mention",name:"文件",target:{kind:"file",sourceId:'context:{"kind":"note","resourceId":"00000000-0000-4000-8000-000000000001"}',resourceId:"00000000-0000-4000-8000-000000000001"}}} onClose={close}/>));
    expect(path).toEqual(["notes", "00000000-0000-4000-8000-000000000001"]);
    expect(document.querySelector("dialog")).toBeNull();expect(close).toHaveBeenCalledOnce();
  } finally {window.removeEventListener(SOURCE_NAVIGATION_EVENT,listener);await act(async()=>root.unmount());}
});

it("reports a missing file only when its tag is opened", async () => {
  const {notify}=await import("../../services/notifications"), error=vi.spyOn(notify,"error");
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host),close=vi.fn();
  try {
    await act(async()=>root.render(<AgentReferenceDocument reference={{type:"mention",name:"缺失文件",target:{kind:"file",sourceId:"missing",resourceId:"missing"}}} onClose={close}/>));
    expect(error).toHaveBeenCalledWith("找不到文件对象");expect(close).toHaveBeenCalledOnce();
  } finally {await act(async()=>root.unmount());}
});
