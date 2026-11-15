/** @vitest-environment happy-dom */
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentEvidenceReferences } from "./AgentEvidenceReferences";
import { AgentUserMessage, AgentUserMessageEvidence } from "./AgentUserMessage";
import { AgentUserMessageScope } from "./AgentUserMessageScope";
import { agentDraftInput, type AgentEvidenceInput } from "@research-assistant/core/agent";
import { useAgentComposerInput } from "./useAgentComposerInput";
import type { AgentComposerInputHandle } from "./AgentComposerTextInput";
import { readEvidenceImage } from "./evidenceImageGateway";
import { navigateSource } from "./sourceNavigation";
vi.mock("./sourceNavigation",()=>({navigateSource:vi.fn(async()=>({getAnchorRect:()=>new DOMRect(50,50,100,20)}))}));
vi.mock("./evidenceImageGateway",()=>({readEvidenceImage:vi.fn(),releaseRemovedEvidence:vi.fn(),retainHistoryEvidence:vi.fn(),releaseHistoryEvidence:vi.fn()}));

it("shows a single-line comment preview while editing retains the full original comment", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const host=document.createElement("div"),root=createRoot(host);document.body.append(host);
  const comment=`第一行\n${"后续评论".repeat(30)}`, change=vi.fn();
  const item:AgentEvidenceInput={type:"annotation",source:"source://notes/note:#block{b}",title:"笔记块",text:"原文",comment};
  try {
    await act(async()=>root.render(<AgentEvidenceReferences evidence={[item]} onChange={change}/>));
    const preview=document.querySelector(".agent-evidence-comment")!.textContent!;
    expect(preview).toContain("第一行 ");expect(preview).not.toContain("\n");expect(preview).toHaveLength(79);expect(preview.endsWith("…")).toBe(true);
    expect(document.querySelector('.agent-evidence-comment [role="img"][aria-label="换行"]')).not.toBeNull();
    await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="编辑批注 笔记块"]')!.click());
    expect(document.querySelector("textarea")?.value).toBe(comment);expect(change).not.toHaveBeenCalled();
  } finally {await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});

it("previews captured image bytes through a disposable object URL", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  const create = vi.spyOn(URL,"createObjectURL").mockReturnValue("blob:evidence-preview"), revoke = vi.spyOn(URL,"revokeObjectURL").mockImplementation(()=>undefined);
  const bytes = Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jGHEAAAAASUVORK5CYII="),character=>character.charCodeAt(0));
  vi.mocked(readEvidenceImage).mockResolvedValue(Uint8Array.from(bytes));
  try {
    await act(async()=>root.render(<AgentEvidenceReferences evidence={[{type:"annotation",source:"source://notes/note:#block{image}",title:"图像快照",text:"",comment:"解释这张图",image:{mediaType:"image/png",assetId:"a".repeat(64),captureId:"00000000-0000-4000-8000-000000000001",byteSize:bytes.length}}]}/>));
    expect(document.querySelector<HTMLImageElement>('[alt="图像快照"]')?.getAttribute("src")).toBe("blob:evidence-preview");
    const blob = create.mock.calls[0]![0] as Blob;
    expect(blob.type).toBe("image/png"); expect(blob.size).toBe(bytes.length);
    await act(async()=>root.unmount());
    expect(revoke).toHaveBeenCalledWith("blob:evidence-preview");
  } finally {host.remove();create.mockRestore();revoke.mockRestore();}
});

it("updates evidence-only edits even when composer text is unchanged", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  function Harness() {
    const [text,setText] = useState(""), input = useRef<AgentComposerInputHandle>(null);
    const composer = useAgentComposerInput({text,setText,input});
    return <><button onClick={()=>composer.changeDraft({text:"",bindings:[],evidence:[{type: "annotation", comment: "",source:"source://notes/note:#block{b}",title:"引用",text:"原文"}]})}>capture</button>
      <AgentEvidenceReferences evidence={composer.draft.evidence ?? []} onChange={evidence=>composer.changeDraft({...composer.draft,evidence})}/></>;
  }
  try {
    await act(async()=>root.render(<Harness/>));
    await act(async()=>host.querySelector<HTMLButtonElement>("button")!.click());
    expect(host.querySelector(".agent-evidence-disclosure")?.textContent).toBe("1 条批注");
    await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="移除批注 引用"]')!.click());
    expect(host.querySelector(".agent-evidence-disclosure")).toBeNull();
  } finally {await act(async()=>root.unmount());host.remove();}
});
it("renders grouped snapshots, editable comments and precise removal, then retains sent evidence", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  const evidence: readonly AgentEvidenceInput[] = [{type: "annotation", comment: "",source:"source://notes/note:#block{b}",title:"笔记块",text:"<script>quoted 🧪"}, {type:"annotation",source:"source://notes/note:#block{c}",title:"另一块",text:"evidence",comment:"用户评论"}, {type:"annotation",source:"source://notes/note:#block{d}",title:"第三块",text:"另一段原文",comment:""}];
  const change = vi.fn();
  try {
    await act(async () => root.render(<AgentEvidenceReferences evidence={evidence} onChange={change}/>));
    expect([...host.querySelectorAll(".agent-evidence-disclosure")].map(item => item.textContent)).toEqual(["3 条批注"]);
    expect(document.querySelector(".agent-evidence-number")).toBeNull();
    expect(document.querySelector(".agent-evidence-target strong")?.textContent).toBe(evidence[0]!.text);
    expect(document.querySelector("textarea")).toBeNull();
    expect(document.querySelector("script")).toBeNull();
    await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="移除批注 笔记块"]')!.click());
    expect(change).toHaveBeenCalledWith([evidence[1], evidence[2]]);
    const message = {id:"user",role:"user" as const,content:"我的问题",userInput:agentDraftInput({text:"我的问题",bindings:[],evidence})};
    await act(async () => root.render(<AgentUserMessageScope message={message} read={undefined}><AgentUserMessageEvidence message={message}/><AgentUserMessage message={message}/></AgentUserMessageScope>));
    expect(host.querySelector("p")?.textContent).toBe("我的问题");
    expect(host.querySelector("textarea")).toBeNull();
    expect(document.querySelector(".agent-evidence-comment")?.textContent).toBe("用户评论");
    expect(host.querySelector('[aria-label^="移除"]')).toBeNull();
  } finally {await act(async () => root.unmount()); host.remove();}
});

it("saves edited comments explicitly and cancels without changing the captured annotation", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  const item: AgentEvidenceInput = {type: "annotation", source: "source://notes/note:#block{b}", title: "笔记块", text: "原文", comment: "旧评论"};
  const change = vi.fn();
  const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(item => item.getAttribute("aria-label") === label || item.textContent === label)!;
  async function type(value: string) {
    const textarea = document.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, value); textarea.dispatchEvent(new Event("input", {bubbles: true}));});
  }
  try {
    await act(async () => root.render(<AgentEvidenceReferences evidence={[item]} onChange={change}/>));
    await act(async () => button("编辑批注 笔记块").click());
    expect(navigateSource).toHaveBeenCalledWith(item.source, {locate:true,annotation:item});
    await type("放弃的修改");
    expect(change).not.toHaveBeenCalled();
    await act(async () => button("取消").click());
    expect(document.querySelector("textarea")).toBeNull();
    expect(document.querySelector(".agent-evidence-comment")?.textContent).toBe("旧评论");
    await act(async () => button("编辑批注 笔记块").click());
    await type("新评论");
    await act(async () => button("保存").click());
    expect(change).toHaveBeenCalledWith([{...item, comment: "新评论"}]);
  } finally {await act(async () => root.unmount()); host.remove();}
});

it("clears all draft annotations with the badge action without opening the preview", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  const change = vi.fn();
  try {
    await act(async () => root.render(<AgentEvidenceReferences evidence={[{type: "annotation", comment: "", source: "source://notes/note", title: "选段", text: "原文"}]} onChange={change}/>));
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="清除全部批注"]')!.click());
    expect(change).toHaveBeenCalledExactlyOnceWith([]);
    expect(host.querySelector(".agent-evidence-disclosure")?.getAttribute("aria-expanded")).toBe("false");
  } finally {await act(async () => root.unmount()); host.remove();}
});

it("selects the whole row and navigates without a separate source button or edit/delete bubbling", async () => {
  vi.mocked(navigateSource).mockClear();
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  const item: AgentEvidenceInput = {type:"annotation",source:"source://notes/note:#block{b}",title:"位置",text:"原文",comment:""};
  const change = vi.fn();
  try {
    await act(async()=>root.render(<AgentEvidenceReferences evidence={[item]} onChange={change}/>));
    const row = document.querySelector<HTMLElement>(".agent-evidence-card")!;
    expect(row.querySelector("a")).toBeNull();
    await act(async()=>row.click());
    expect(row.classList.contains("is-selected")).toBe(true);
    expect(row.getAttribute("aria-current")).toBe("location");
    expect(navigateSource).toHaveBeenCalledExactlyOnceWith(item.source,{annotation:item});
    await act(async()=>row.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true})));
    expect(navigateSource).toHaveBeenCalledTimes(2);
    await act(async()=>row.querySelector<HTMLButtonElement>('[aria-label^="移除"]')!.click());
    expect(navigateSource).toHaveBeenCalledTimes(2);
    expect(change).toHaveBeenCalledWith([]);
  } finally {await act(async()=>root.unmount()); host.remove();}
});

it("opens the editor only after a real location acknowledgment and discards a superseded capture", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  let located!: (value: {getAnchorRect:()=>DOMRect}) => void;
  vi.mocked(navigateSource).mockImplementationOnce(()=>new Promise(resolve=>{located=resolve;}));
  const item: AgentEvidenceInput = {type:"annotation",source:"source://notes/note:#block{b}",title:"位置",text:"原文",comment:"旧评论"};
  const change=vi.fn();
  try {
    await act(async()=>root.render(<AgentEvidenceReferences evidence={[item]} onChange={change}/>));
    await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label^="编辑批注"]')!.click());
    expect(document.querySelector("textarea")).toBeNull();
    await act(async()=>root.render(<AgentEvidenceReferences evidence={[{...item,text:"另一处"}]} onChange={change}/>));
    await act(async()=>located({getAnchorRect:()=>new DOMRect(50,50,100,20)}));
    expect(document.querySelector("textarea")).toBeNull();
    expect(change).not.toHaveBeenCalled();
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("shows a source-free conversation quote in the shared annotation group and edits it without navigation", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  vi.mocked(navigateSource).mockClear();
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  const item: AgentEvidenceInput = {type:"annotation",title:"对话选段",text:"聊天中的原文\n第二行",comment:""};
  let evidence: readonly AgentEvidenceInput[] = [item];
  function Harness() {
    const [value, setValue] = useState(evidence);
    return <AgentEvidenceReferences evidence={value} onChange={next => {evidence = next; setValue(next);}}/>;
  }
  try {
    await act(async()=>root.render(<Harness/>));
    expect(host.querySelector(".agent-evidence-disclosure")?.textContent).toBe("1 条批注");
    await act(async()=>host.querySelector<HTMLButtonElement>(".agent-evidence-disclosure")!.click());
    const row = document.querySelector<HTMLElement>('.agent-evidence-card[data-unlocated]')!;
    expect(row.querySelector("blockquote")?.textContent).toBe(item.text);
    await act(async()=>row.click());
    expect(navigateSource).not.toHaveBeenCalled();
    const edit = row.querySelector<HTMLButtonElement>('[aria-label="编辑批注 对话选段"]')!;
    await act(async()=>{edit.dispatchEvent(new PointerEvent("pointerdown",{bubbles:true,button:0})); edit.dispatchEvent(new MouseEvent("mousedown",{bubbles:true,button:0})); edit.dispatchEvent(new PointerEvent("pointerup",{bubbles:true,button:0})); edit.dispatchEvent(new MouseEvent("mouseup",{bubbles:true,button:0})); edit.click();});
    expect(navigateSource).not.toHaveBeenCalled();
    const textarea = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    expect(textarea).not.toBeNull();
    await act(async()=>{textarea.dispatchEvent(new PointerEvent("pointerdown",{bubbles:true,button:0})); textarea.dispatchEvent(new MouseEvent("mousedown",{bubbles:true,button:0})); textarea.focus(); textarea.dispatchEvent(new PointerEvent("pointerup",{bubbles:true,button:0})); textarea.dispatchEvent(new MouseEvent("mouseup",{bubbles:true,button:0})); textarea.click();});
    expect(document.querySelector('textarea[aria-label="批注 对话选段"]')).not.toBeNull();
    await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea,"补充说明"); textarea.dispatchEvent(new Event("input",{bubbles:true}));});
    const save = [...document.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent === "保存")!;
    await act(async()=>{save.dispatchEvent(new PointerEvent("pointerdown",{bubbles:true,button:0})); save.dispatchEvent(new MouseEvent("mousedown",{bubbles:true,button:0})); save.dispatchEvent(new PointerEvent("pointerup",{bubbles:true,button:0})); save.dispatchEvent(new MouseEvent("mouseup",{bubbles:true,button:0})); save.click();});
    expect(evidence).toEqual([{...item, comment:"补充说明"}]);
    await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="移除批注 对话选段"]')!.click());
    expect(evidence).toEqual([]);
    expect(navigateSource).not.toHaveBeenCalled();
    const message = {id:"user",role:"user" as const,content:"我的问题",userInput:agentDraftInput({text:"我的问题",bindings:[],evidence:[item]})};
    await act(async()=>root.render(<AgentUserMessageScope message={message} read={undefined}><AgentUserMessageEvidence message={message}/><AgentUserMessage message={message}/></AgentUserMessageScope>));
    expect(host.querySelector(".agent-evidence-disclosure")?.textContent).toBe("1 条批注");
    expect(document.querySelector<HTMLButtonElement>('[aria-label="编辑批注 对话选段"]')).toBeNull();
    expect(document.querySelector<HTMLButtonElement>('[aria-label="移除批注 对话选段"]')).toBeNull();
  } finally {await act(async()=>root.unmount()); host.remove();}
});
