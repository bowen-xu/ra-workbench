// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { afterEach, expect, it, vi } from "vitest";
import { getAppStorage, selectAppStorage } from "../../runtime/appStorage";
import { getReaderCloseCoordinator } from "../../workspace/DocumentCloseCoordinator";
import { WorkspaceTabBar, WorkspaceTabsProvider } from "../workspace-tabs/WorkspaceTabs";
import { openTextDocument, type TextDocumentRequest } from "./textDocumentEvents";
import { useTextDocuments } from "./useTextDocuments";
import {notify} from "../../services/notifications";
import { registerStoragePersistence } from "../../runtime/storagePersistence";
import {systemDocumentGateway} from "../../services/systemDocumentGateway";

vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn() } }));
let root: ReturnType<typeof createRoot> | undefined, host: HTMLDivElement;
afterEach(async () => { if (root) await act(async () => root!.unmount()); host?.remove(); });
async function harness(memory = new Map<string, string>()) {
  selectAppStorage({ get length() { return memory.size; }, getItem: key => memory.get(key) ?? null, setItem: (key, value) => { memory.set(key, value); }, removeItem: key => { memory.delete(key); }, key: index => [...memory.keys()][index] ?? null, clear: () => memory.clear() });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  let documents!: ReturnType<typeof useTextDocuments>;
  function Shell() {
    documents = useTextDocuments("notes");
    return <><WorkspaceTabBar owner="notes" textActive={documents.activeId !== undefined} navigate={(_, __, ready) => { ready(); return true; }} />{documents.surface}</>;
  }
  await act(async () => root!.render(<WorkspaceTabsProvider><Shell /></WorkspaceTabsProvider>));
  return { get documents() { return documents; }, open: async (request: TextDocumentRequest) => { await act(async () => { expect(openTextDocument(request)).toBe(true); }); } };
}
function editor(index = 0): EditorView { return EditorView.findFromDOM(host.querySelectorAll<HTMLElement>(".cm-editor")[index]!)!; }
it("restores registered text tabs through the gateway and keeps editable recovery text",async()=>{
  const memory=new Map<string,string>();
  const request:TextDocumentRequest={id:"system:structured-task",path:"source://agents/prompts/structured-task.md",content:"current",owner:"notes",reopen:{kind:"system",id:"structured-task"},save:vi.fn()};
  const read=vi.spyOn(systemDocumentGateway,"read").mockResolvedValue({id:"structured-task",name:"Instructions",workflow:"shared",path:request.path,content:"current",baseline:"current",revision:"v1"});
  try {
    const first=await harness(memory);await first.open(request);
    await act(async()=>{editor().dispatch({changes:{from:0,to:7,insert:"my edit"}});});
    await act(async()=>root!.unmount());root=undefined;host.remove();
    await harness(memory);
    expect(read).toHaveBeenCalledExactlyOnceWith("structured-task");
    expect(host.querySelectorAll(".text-document-host")).toHaveLength(1);
    expect(editor().state.doc.toString()).toBe("my edit");
    expect(host.querySelector('[aria-selected="true"]')?.textContent).toContain("Instructions");
    // The tab manifest carries identity, not a second copy of the document.
    expect(memory.get("research-assistant:workspace.v1:text-tabs")).not.toContain("my edit");
  } finally {read.mockRestore();}
});

it("does not rewrite unchanged recovery snapshots on repeated workspace preparation", async () => {
  const memory = new Map<string, string>(), h = await harness(memory);
  for(let id = 0; id < 4; id++) await h.open({id: `perf-${id}`, path: `perf-${id}.md`, content: "synthetic saved text", save: vi.fn()});
  const writes = vi.spyOn(memory, "set");
  try {
    for(let visit = 0; visit < 20; visit++) expect(await getReaderCloseCoordinator().prepareAll()).toBe(true);
    if (process.env.WORKSPACE_PERF_REPORT === "1") process.stdout.write(`workspace text recovery writes=${writes.mock.calls.length} documents=4 visits=20\n`);
    expect(writes).toHaveBeenCalledTimes(4);
    memory.delete("research-assistant:text-draft:perf-0");
    expect(await getReaderCloseCoordinator().prepareAll()).toBe(true); expect(writes).toHaveBeenCalledTimes(5);
  } finally {writes.mockRestore();}
});

it("blocks preparation when durable storage cannot be rechecked and preserves diagnostic identity", async () => {
  const memory = new Map<string, string>(), h = await harness(memory);
  await h.open({id: "checkpoint-error", path: "checkpoint-error.md", content: "saved", save: vi.fn()});
  expect(await getReaderCloseCoordinator().prepareAll()).toBe(true);
  const read = vi.spyOn(memory, "get").mockImplementation(() => {throw new DOMException("blocked", "SecurityError");});
  try {
    expect(await getReaderCloseCoordinator().prepareAll()).toBe(false);
    expect(notify.error).toHaveBeenLastCalledWith(expect.stringContaining("text_recovery_checkpoint_failed；文档 checkpoint-error"));
    expect(notify.error).toHaveBeenLastCalledWith(expect.stringContaining("SecurityError: blocked"));
  } finally {read.mockRestore();}
});

it("blocks text workspace preparation when only the staged checkpoint succeeds but its async receipt fails", async () => {
  const memory = new Map<string, string>(), h = await harness(memory);
  await h.open({id: "async-draft", path: "draft.md", content: "retained text"});
  const failure = new Error("preview_storage_failed: native disk full");
  const flush = vi.fn<() => Promise<void>>().mockRejectedValue(failure);
  registerStoragePersistence(getAppStorage(), {backend: "IndexedDB", pending: true, error: undefined, resetting: false,
    flush, retry: flush, reset: flush, subscribe: () => () => {}, close() {}});
  await act(async () => expect(await getReaderCloseCoordinator().prepareAll()).toBe(false));
  expect(memory.get("research-assistant:text-draft:async-draft")).toContain("retained text");
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("native disk full"));
  expect(h.documents.activeId).toBe("async-draft");
  flush.mockResolvedValue(undefined);
  await act(async () => expect(await getReaderCloseCoordinator().prepareAll()).toBe(true));
});

it("refreshes a clean open document and its revision-bound save callback before reviewing an external edit",async()=>{
  const h=await harness(), staleSave=vi.fn(), currentSave=vi.fn();
  const request={id:"system:structured-task",path:"source://agents/prompts/structured-task.md"};
  await h.open({...request,content:"before",save:staleSave});
  const view=editor();
  await h.open({...request,content:"after",baseline:"before",save:currentSave});
  expect(editor()).toBe(view);
  expect(host.querySelectorAll(".cm-editor")).toHaveLength(1);
  expect(view.state.doc.toString()).toBe("after");
  expect(host.querySelector('[aria-label="当前变更位置"]')).not.toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="Undo all"]')!.click());
  expect(view.state.doc.toString()).toBe("before");
  expect(currentSave).toHaveBeenCalledWith("before","before");
  expect(staleSave).not.toHaveBeenCalled();
});
it("keeps unsaved edits and their original version guard when an updated source is opened",async()=>{
  const h=await harness(), originalSave=vi.fn(), newerSave=vi.fn();
  const request={id:"rules",path:"source://agents/prompts/rules.md"};
  await h.open({...request,content:"before",save:originalSave});
  await act(async()=>editor().dispatch({changes:{from:6,insert:" local"},userEvent:"input.type"}));
  await h.open({...request,content:"external",baseline:"before",save:newerSave});
  expect(editor().state.doc.toString()).toBe("before local");
  await act(async()=>host.querySelector<HTMLElement>(".text-editor")!.dispatchEvent(new KeyboardEvent("keydown",{key:"s",metaKey:true,bubbles:true,cancelable:true})));
  expect(originalSave).toHaveBeenCalledWith("before local","before local");
  expect(newerSave).not.toHaveBeenCalled();
});

it("keeps file history and dirty edits across tabs, closes durably, and restores by stable identity", async () => {
  const h = await harness();
  await h.open({ id: "a", path: "a.md", content: "original", save: vi.fn() });
  const first = editor();
  await act(async () => first.dispatch({ changes: { from: 8, insert: " edited" }, userEvent: "input.type" }));
  await h.open({ id: "b", path: "b.md", content: "second" });
  expect(editor()).toBe(first); expect(h.documents.activeId).toBe("b");
  await h.open({ id: "a", path: "a.md", content: "original" });
  expect(first.state.doc.toString()).toBe("original edited");
  let closing!: Promise<boolean>;
  await act(async () => { closing = getReaderCloseCoordinator().close("text:a"); });
  expect(document.querySelector('[aria-label="保存文本修改"]')).not.toBeNull();
  await act(async () => document.querySelector<HTMLButtonElement>(".text-save-dialog button:last-child")!.click());
  expect(await closing).toBe(true);
  expect(host.querySelectorAll(".cm-editor")).toHaveLength(1);
  await h.open({ id: "a", path: "a.md", content: "original edited" });
  expect(editor(1).state.doc.toString()).toBe("original edited");
});
it("keeps failed saves editable and preserves typing made during a slow save", async () => {
  const h = await harness();
  let complete!: () => void;
  const save = vi.fn().mockRejectedValueOnce(new Error("offline")).mockImplementationOnce(() => new Promise<void>(resolve => { complete = resolve; }));
  await h.open({ id: "saved", path: "saved.md", content: "old", save });
  const view = editor();
  await act(async () => view.dispatch({ changes: { from: 3, insert: " new" }, userEvent: "input.type" }));
  await act(async () => host.querySelector<HTMLElement>(".text-editor")!.dispatchEvent(new KeyboardEvent("keydown", { key: "s", metaKey: true, bubbles: true, cancelable: true })));
  expect(host.querySelector('[aria-label="未保存"]')).not.toBeNull();
  await act(async () => host.querySelector<HTMLElement>(".text-editor")!.dispatchEvent(new KeyboardEvent("keydown", { key: "s", metaKey: true, bubbles: true, cancelable: true })));
  await act(async () => view.dispatch({ changes: { from: 7, insert: " later" }, userEvent: "input.type" }));
  await act(async () => complete());
  expect(view.state.doc.toString()).toBe("old new later");
  expect(host.querySelector('[aria-label="未保存"]')).not.toBeNull();
  const draft = JSON.parse(getAppStorage().getItem("research-assistant:text-draft:saved")!);
  expect(draft.saved).toBe("old new"); expect(draft.content).toBe("old new later");
});
it("automatically saves a supplied Diff resolution using the shared toolbar", async () => {
  const h = await harness(), save = vi.fn();
  await h.open({ id: "diff", path: "rules.md", content: "after", baseline: "before", save });
  expect(host.querySelector('[aria-label="当前变更位置"]')?.textContent).toBe("1/1");
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Undo all"]')!.click());
  expect(editor().state.doc.toString()).toBe("before"); expect(save).toHaveBeenCalledOnce();
});
it("restores a workspace's last text tab after a section switch without reopening closed text", async () => {
  const h = await harness();
  await h.open({ id: "remembered", path: "rules.md", content: "rules" });
  const restore = h.documents.restoreOwner("notes");
  await act(async () => { h.documents.deactivate(); h.documents.forgetOwner("notes"); });
  expect(h.documents.activeId).toBeUndefined();
  await act(async () => restore());
  expect(h.documents.activeId).toBe("remembered");
  const closedRestore = h.documents.restoreOwner("notes");
  await act(async () => { await getReaderCloseCoordinator().close("text:remembered"); });
  await act(async () => closedRestore());
  expect(h.documents.activeId).toBeUndefined();
});
it("compares a recovery draft with changed source content without writing the source", async () => {
  const h = await harness(), save = vi.fn();
  getAppStorage().setItem("research-assistant:text-draft:changed", JSON.stringify({ content: "local draft", baseline: "old", saved: "old" }));
  await h.open({ id: "changed", path: "changed.md", content: "external version", save });
  expect(editor().state.doc.toString()).toBe("local draft");
  expect(host.querySelector('[aria-label="当前变更位置"]')).not.toBeNull();
  expect(save).not.toHaveBeenCalled();
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Undo all"]')!.click());
  expect(editor().state.doc.toString()).toBe("external version");
});
it("prompts only on close, supports cancel and discard, and does not save manual edits", async () => {
  const h = await harness(), save = vi.fn();
  await h.open({ id: "close", path: "close.md", content: "old", save });
  await act(async () => editor().dispatch({ changes: { from: 3, insert: " draft" }, userEvent: "input.type" }));
  expect(save).not.toHaveBeenCalled();
  expect(host.querySelector('[aria-label="撤销编辑"]')).toBeNull();
  expect(host.querySelector('[aria-label="预览 Markdown"]')).toBeNull();
  await act(async () => { expect(await getReaderCloseCoordinator().prepareAll()).toBe(true); });
  expect(document.querySelector('[aria-label="保存文本修改"]')).toBeNull();
  let closing!: Promise<boolean>;
  await act(async () => { closing = getReaderCloseCoordinator().close("text:close"); });
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="取消关闭文本"]')!.click());
  expect(await closing).toBe(false);
  expect(editor().state.doc.toString()).toBe("old draft");
  await act(async () => { closing = getReaderCloseCoordinator().close("text:close"); });
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="不保存并关闭文本"]')!.click());
  expect(await closing).toBe(true);
  await h.open({ id: "close", path: "close.md", content: "old", save });
  expect(editor().state.doc.toString()).toBe("old");
  expect(save).not.toHaveBeenCalled();
});
it("recovers unsaved content and pending Diff and retains a failed automatic Keep save", async () => {
  const h = await harness(), save = vi.fn().mockRejectedValue(new Error("offline"));
  getAppStorage().setItem("research-assistant:text-draft:recovery", JSON.stringify({ content: "draft", baseline: "before", saved: "source", savedBaseline: "before" }));
  await h.open({ id: "recovery", path: "recovery.md", content: "source", baseline: "before", save });
  expect(editor().state.doc.toString()).toBe("draft");
  expect(host.querySelector('[aria-label="未保存"]')).not.toBeNull();
  expect(host.querySelector('[aria-label="当前变更位置"]')).not.toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="Keep all"]')!.click());
  expect(save).toHaveBeenCalledWith("draft", "draft");
  expect(host.querySelector('[aria-label="未保存"]')).not.toBeNull();
  const draft = JSON.parse(getAppStorage().getItem("research-assistant:text-draft:recovery")!);
  expect(draft.content).toBe("draft"); expect(draft.baseline).toBe("draft"); expect(draft.saved).toBe("source");
});

it("recovers the unsaved document after an unprompted host shutdown", async () => {
  const memory = new Map<string, string>(), save = vi.fn();
  const first = await harness(memory);
  await first.open({ id: "restart", path: "restart.md", content: "incoming", baseline: "before", save });
  await act(async () => editor().dispatch({ changes: { from: 8, insert: " local" }, userEvent: "input.type" }));
  await act(async () => root!.unmount()); host.remove();
  const second = await harness(memory);
  await second.open({ id: "restart", path: "restart.md", content: "incoming", baseline: "before", save });
  expect(editor().state.doc.toString()).toBe("incoming local");
  expect(host.querySelector('[aria-label="未保存"]')).not.toBeNull();
  expect(host.querySelector('[aria-label="当前变更位置"]')).not.toBeNull();
  expect(save).not.toHaveBeenCalled();
});

it("reveals documents only after destination navigation accepts and activates them",async()=>{
  const {TEXT_DOCUMENT_REVEALED}=await import("./textDocumentEvents");
  const revealed=vi.fn();window.addEventListener(TEXT_DOCUMENT_REVEALED,revealed);
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
  let ready:(()=>void)|undefined;
  const navigate=vi.fn((_owner:unknown,activate:()=>void)=>{ready=activate;return false;});
  function Shell(){useTextDocuments("notes",navigate);return null;}
  try{
    await act(async()=>root!.render(<Shell/>));
    await act(async()=>{expect(openTextDocument({id:"rejected",path:"a.md",content:"text"})).toBe(false);});
    expect(revealed).not.toHaveBeenCalled();
    navigate.mockImplementation((_owner,activate)=>{ready=activate;return true;});
    await act(async()=>{expect(openTextDocument({id:"accepted",path:"a.md",content:"text"})).toBe(true);});
    expect(revealed).not.toHaveBeenCalled();
    await act(async()=>ready!());expect(revealed).toHaveBeenCalledOnce();
  }finally{window.removeEventListener(TEXT_DOCUMENT_REVEALED,revealed);}
});
