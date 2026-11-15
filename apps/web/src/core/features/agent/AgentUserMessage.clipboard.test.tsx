/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {insertAgentReference, serializeAgentSourceReferences, type AgentInputDraft, type AgentReferenceInput} from "@research-assistant/core/agent";
import {AgentUserMessage} from "./AgentUserMessage";
import {AgentUserMessageScope} from "./AgentUserMessageScope";
import {AgentMessageCopyButton} from "./AgentMessageCopyButton";
import {readComposerClipboard} from "./agentComposerClipboard";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => {document.body.replaceChildren(); document.getSelection()?.removeAllRanges(); vi.restoreAllMocks();});
const file: AgentReferenceInput = {type: "mention", name: "同名文件", target: {kind: "file", sourceId: "paper", resourceId: "first", revision: "1"}};
const other: AgentReferenceInput = {...file, target: {...file.target as Extract<typeof file.target, {kind:"file"}>, resourceId: "second", revision: "2"}};
const draft = [file, other].reduce((value, input) => insertAgentReference(value, input), {text: "🧪 前文\n", bindings: []} as AgentInputDraft);
function clipboard() {
  const values = new Map<string,string>();
  return {getData: (type:string) => values.get(type) ?? "", setData: (type:string, value:string) => {values.set(type,value);}};
}
async function mount(legacy = false) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const wire = serializeAgentSourceReferences(draft, {kind:"manuscript",resourceId:"paper",conversationId:"chat"});
  const message = {id: "turn/user", role: "user" as const, content: wire.text, ...(legacy ? {rawData: {}} : {userInput: wire.userInput})};
  const read = vi.fn(async () => ({userInput: wire.userInput}));
  await act(async () => root.render(<AgentUserMessageScope message={message} read={read}><div className="writing-agent-transcript"><article className="writing-agent-message user"><div className="writing-agent-message-bubble"><AgentUserMessage message={message} read={read}/></div><AgentMessageCopyButton value={wire.text} messageId={message.id}/></article><article className="writing-agent-message assistant"><div className="writing-agent-message-bubble"><p>回答开始</p><p>回答结束</p></div></article></div></AgentUserMessageScope>));
  return {host, read, close: () => act(async () => root.unmount())};
}
it("whole-message copying reuses the HTML carrier and one frozen metadata read", async () => {
  const data = clipboard();
  Object.defineProperty(navigator, "clipboard", {configurable:true, value:{}});
  const exec = vi.fn(() => {
    const event = new Event("copy", {bubbles:true,cancelable:true}); Object.defineProperty(event,"clipboardData",{value:data});
    document.dispatchEvent(event); return true;
  });
  Object.defineProperty(document,"execCommand",{configurable:true,value:exec});
  const view = await mount(true);
  try {
    expect(view.read).toHaveBeenCalledOnce();
    await act(async () => view.host.querySelector<HTMLButtonElement>('[aria-label="复制消息"]')!.click());
    expect(readComposerClipboard(data)).toEqual({...draft,text:draft.text.trimEnd()});
    expect(view.host.querySelector('[aria-label="已复制消息"]')).not.toBeNull();
  } finally {await view.close();}
});
it("DOM selection copies Unicode and distinct same-name identities, with partial tags atomic", async () => {
  const view = await mount();
  try {
    const p = view.host.querySelector("p")!, selection = document.getSelection()!;
    const copy = () => {
      const data = clipboard(), event = new Event("copy", {bubbles:true,cancelable:true}); Object.defineProperty(event,"clipboardData",{value:data});
      document.dispatchEvent(event); return {data,event};
    };
    const range = document.createRange(); range.selectNodeContents(p); selection.addRange(range);
    expect(readComposerClipboard(copy().data)).toEqual({...draft,text:draft.text.trimEnd()});
    selection.removeAllRanges();
    const label = p.querySelector("[data-agent-copy-reference] .inline-reference-label") ?? p.querySelector("[data-agent-copy-reference] button span:last-child");
    const node = label!.firstChild!;
    range.setStart(node,1); range.setEnd(node,2); selection.addRange(range);
    expect(readComposerClipboard(copy().data)?.bindings.map(binding => binding.input)).toEqual([file]);
    selection.removeAllRanges(); range.setStart(p.firstElementChild!.firstChild!,0); range.setEnd(p.firstElementChild!.firstChild!,2); selection.addRange(range);
    expect(copy().event.defaultPrevented).toBe(false);
  } finally {await view.close();}
});

it("cross-message selections covering the ellipsis copy the complete user message and preserve tags", async () => {
  const height=vi.spyOn(HTMLElement.prototype,"scrollHeight","get").mockReturnValue(400);
  const bounds=vi.spyOn(HTMLElement.prototype,"getBoundingClientRect").mockReturnValue(new DOMRect(0,0,600,200));
  const style=vi.spyOn(globalThis,"getComputedStyle").mockReturnValue({lineHeight:"20px"} as CSSStyleDeclaration);
  const view=await mount();
  try {
    const p=view.host.querySelector("p")!, last=view.host.querySelector(".assistant p:last-child")!;
    const range=document.createRange();range.setStart(p.firstElementChild!.firstChild!,3);range.setEnd(last.firstChild!,2);
    document.getSelection()!.addRange(range);
    const data=clipboard(), event=new Event("copy",{bubbles:true,cancelable:true});Object.defineProperty(event,"clipboardData",{value:data});
    document.dispatchEvent(event);
    const result=readComposerClipboard(data)!;
    expect(result.text).toBe(draft.text.trimEnd()+"\n\n回答开始\n回答");
    expect(result.bindings).toEqual(draft.bindings);
    expect(result.text).not.toContain("...");expect(result.text).not.toContain("展开");
    expect(event.defaultPrevented).toBe(true);
  } finally {await view.close();height.mockRestore();style.mockRestore();bounds.mockRestore();}
});
