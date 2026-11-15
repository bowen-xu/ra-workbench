/** @vitest-environment happy-dom */
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { undo, redo } from "@codemirror/commands";
import { afterEach, expect, it, vi } from "vitest";
import { insertAgentReference, type AgentInputDraft, type AgentReferenceInput } from "@research-assistant/core/agent";
import { AgentComposerTextInput, type AgentComposerInputHandle } from "./AgentComposerTextInput";
import { notify } from "../../services/notifications";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => {document.body.replaceChildren(); vi.restoreAllMocks();});
const skill: AgentReferenceInput = {type: "skill", name: "同名技能", identity: {environment_id: "manuscript:test", source_id: "first/SKILL.md", revision: "2"}};
const otherSkill: AgentReferenceInput = {...skill, identity: {...skill.identity, source_id: "second/SKILL.md", revision: "7"}};
const file: AgentReferenceInput = {type: "mention", name: "main.tex", target: {kind: "file", sourceId: "writing:test", resourceId: "main.tex", revision: "file-v1"}};
const tool: AgentReferenceInput = {type: "mention", name: "文献检索", target: {kind: "tool", serverId: "literature", environmentId: "mock", revision: "tool-v1"}};
const fullDraft = [skill, file, tool, otherSkill].reduce((draft, input) => insertAgentReference(draft, input), {text: "😀 前文\n", bindings: []} as AgentInputDraft);
function Harness({initial}: {initial: AgentInputDraft}) {
  const [draft, setDraft] = useState(initial), input = useRef<AgentComposerInputHandle>(null);
  return <><AgentComposerTextInput inputRef={input} aria-label="消息" draft={draft} onDraftChange={setDraft}
    onChange={text => setDraft({text, bindings: []})} onCaretChange={() => {}} onKeyDown={() => {}} />
    <output>{JSON.stringify(draft)}</output></>;
}
async function mount(initial = fullDraft) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<Harness initial={initial} />));
  return {host, draft: () => JSON.parse(host.querySelector("output")!.textContent!) as AgentInputDraft,
    close: () => act(async () => root.unmount())};
}
function editor(host: HTMLElement) { return EditorView.findFromDOM(host.querySelector<HTMLElement>(".cm-editor")!)!; }
function clipboard() {
  const values = new Map<string, string>();
  return {getData: (type: string) => values.get(type) ?? "", setData: (type: string, value: string) => {values.set(type, value);},
    clearData: () => values.clear(), values};
}
async function dispatch(target: HTMLElement, type: "copy" | "cut" | "paste", data: ReturnType<typeof clipboard>) {
  const event = new Event(type, {bubbles: true, cancelable: true}); Object.defineProperty(event, "clipboardData", {value: data});
  await act(async () => target.dispatchEvent(event)); return event;
}

it("cut-all and paste restore Unicode, all three reference types and same-name identities, with undo/redo", async () => {
  const view = await mount(); try {
    const cm = editor(view.host), data = clipboard();
    await act(async () => {cm.focus(); cm.dispatch({selection: {anchor: 0, head: cm.state.doc.length}});});
    await dispatch(cm.contentDOM, "cut", data);
    expect(view.draft()).toEqual({text: "", bindings: []});
    expect(data.getData("text/plain")).toBe(fullDraft.text);
    await dispatch(editor(view.host).contentDOM, "paste", data);
    expect(view.draft()).toEqual(fullDraft);
    expect(view.host.querySelectorAll(".agent-composer-reference")).toHaveLength(4);
    await act(async () => undo(cm)); expect(view.draft().text).toBe("");
    await act(async () => redo(cm)); expect(view.draft()).toEqual(fullDraft);
  } finally {await view.close();}
});

it.each(["empty", "replacement"])("HTML-only clipboard restores references in a %s native textarea and undo restores its original text", async mode => {
  const source = await mount(), data = clipboard();
  try {
    const cm = editor(source.host); await act(async () => cm.dispatch({selection: {anchor: 0, head: cm.state.doc.length}}));
    await dispatch(cm.contentDOM, "copy", data);
  } finally {await source.close();}
  for (const type of data.values.keys()) if (type !== "text/plain" && type !== "text/html") data.values.delete(type);
  const initial = {text: mode === "empty" ? "" : "前缀 被替换 后缀", bindings: []};
  const target = await mount(initial); try {
    const input = target.host.querySelector("textarea")!;
    input.focus(); input.setSelectionRange(mode === "empty" ? 0 : 3, mode === "empty" ? 0 : 6);
    await dispatch(input, "paste", data);
    const restored = target.draft(), cm = editor(target.host);
    expect(restored.text).toBe(mode === "empty" ? fullDraft.text : "前缀 " + fullDraft.text + " 后缀");
    expect(restored.bindings.map(binding => binding.input)).toEqual(fullDraft.bindings.map(binding => binding.input));
    expect(cm.state.selection.main.head).toBe((mode === "empty" ? 0 : 3) + fullDraft.text.length);
    expect(cm.hasFocus).toBe(true);
    // happy-dom fires selectionchange synchronously during DOM replacement;
    // browser-focused keyboard history is covered by the real UI check.
    cm.contentDOM.blur();
    await act(async () => undo(cm)); expect(target.draft()).toEqual(initial);
    await act(async () => redo(cm)); expect(target.draft()).toEqual(restored);
  } finally {await target.close();}
});

it("partial cut copies the whole reference with fragment-relative offsets and retains surrounding bindings", async () => {
  const view = await mount(); try {
    const cm = editor(view.host), binding = fullDraft.bindings[1]!, data = clipboard();
    await act(async () => cm.dispatch({selection: {anchor: binding.startUtf16 + 2, head: binding.endUtf16 - 1}}));
    await dispatch(cm.contentDOM, "cut", data);
    expect(data.getData("text/plain")).toBe("@main.tex"); expect(view.draft().bindings).toHaveLength(3);
    await act(async () => cm.dispatch({selection: {anchor: 0}})); await dispatch(cm.contentDOM, "paste", data);
    expect(view.draft().bindings[0]).toEqual({startUtf16: 0, endUtf16: 9, input: file});
    expect(view.draft().bindings).toHaveLength(4);
  } finally {await view.close();}
});

it("replacing identical token text restores the pasted identity and undo restores the previous identity", async () => {
  const source = await mount(insertAgentReference({text: "", bindings: []}, otherSkill)), data = clipboard();
  try {
    const cm = editor(source.host), binding = source.draft().bindings[0]!;
    await act(async () => cm.dispatch({selection: {anchor: binding.startUtf16, head: binding.endUtf16}}));
    await dispatch(cm.contentDOM, "copy", data);
  } finally {await source.close();}
  const target = await mount(insertAgentReference({text: "", bindings: []}, skill)); try {
    const cm = editor(target.host), binding = target.draft().bindings[0]!;
    await act(async () => cm.dispatch({selection: {anchor: binding.startUtf16, head: binding.endUtf16}}));
    await dispatch(cm.contentDOM, "paste", data); expect(target.draft().bindings[0]!.input).toEqual(otherSkill);
    await act(async () => undo(cm)); expect(target.draft().bindings[0]!.input).toEqual(skill);
  } finally {await target.close();}
});

it("clipboard failure never deletes a cut; ordinary pasted names do not acquire identities", async () => {
  const error = vi.spyOn(notify, "error").mockReturnValue(""); const view = await mount(); try {
    const cm = editor(view.host), data = clipboard();
    await act(async () => cm.dispatch({selection: {anchor: 0, head: cm.state.doc.length}}));
    data.setData = () => {throw new Error("Clipboard unavailable");};
    await dispatch(cm.contentDOM, "cut", data); expect(view.draft()).toEqual(fullDraft); expect(error).toHaveBeenCalledOnce();
    const plain = clipboard(); plain.setData("text/plain", "@同名技能");
    await dispatch(cm.contentDOM, "paste", plain); expect(view.draft()).toEqual({text: "@同名技能", bindings: []});
  } finally {await view.close();}
});

it.each(["version", "overlap", "offset", "surrogate", "identity", "name", "mismatch", "oversize"])("rejects %s clipboard metadata and falls back to plain text", async failure => {
  const error = vi.spyOn(notify, "error").mockReturnValue(""); const view = await mount(); try {
    const cm = editor(view.host), data = clipboard();
    await act(async () => cm.dispatch({selection: {anchor: 0, head: cm.state.doc.length}})); await dispatch(cm.contentDOM, "copy", data);
    const type = [...data.values.keys()].find(type => type.startsWith("application/"))!;
    const payload = JSON.parse(data.getData(type));
    if (failure === "version") payload.version = 99;
    if (failure === "overlap") payload.input[0].textElements[1].startUtf16 = payload.input[0].textElements[0].startUtf16;
    if (failure === "offset") payload.input[0].textElements[0].endUtf16 = 9999;
    if (failure === "surrogate") payload.input[0].textElements[0].startUtf16 = 1;
    if (failure === "identity") payload.input[1].identity.revision = "";
    if (failure === "name") payload.input[1].name = "另一个技能";
    if (failure === "mismatch") data.setData("text/plain", "普通文本");
    data.setData(type, failure === "oversize" ? "x".repeat(2_097_153) : JSON.stringify(payload));
    await dispatch(cm.contentDOM, "paste", data);
    expect(view.draft().bindings).toEqual([]); expect(view.draft().text).toBe(data.getData("text/plain")); expect(error).toHaveBeenCalledOnce();
  } finally {await view.close();}
});
it("returns focus to the chat input when its annotation badge is cleared",async()=>{
  const view=await mount({text:"保留问题",bindings:[],evidence:[{type:"annotation",source:"source://notes/note:#block{b}",title:"引用",text:"原文",comment:""}]});
  try{
    const clear=view.host.querySelector<HTMLButtonElement>('[aria-label="清除全部批注"]')!;
    clear.focus();await act(async()=>clear.click());
    expect(document.activeElement).toBe(view.host.querySelector("textarea"));
    expect(view.draft().text).toBe("保留问题");expect(view.draft().evidence).toEqual([]);
  }finally{await view.close();}
});
