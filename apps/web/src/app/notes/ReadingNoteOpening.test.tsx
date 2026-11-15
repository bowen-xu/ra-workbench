// @vitest-environment happy-dom
import { act, StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import {NotesGatewayError,type NoteDocumentSnapshot} from "@research-assistant/data/notes";
import { DeveloperModeProvider, DeveloperModeToggle } from "../../core/features/settings/DeveloperMode";
import {useAgentContextCandidate} from "../../core/features/agent/context/AgentContextProvider";
import { ReadingNoteFloatingPanel } from "./ReadingNoteFloatingPanel";
import { NotesWorkspace as ManagedNotesWorkspace } from "@research-assistant/note-manager-plugin/workspace";
import { BuiltinPluginConfigurationTest } from "../builtinPluginConfiguration.testSupport";
function NotesWorkspace(props: Parameters<typeof ManagedNotesWorkspace>[0]) {
  return <BuiltinPluginConfigurationTest owner="note_manager"><ManagedNotesWorkspace {...props} /></BuiltinPluginConfigurationTest>;
}
import {selectAppStorage} from "../../core/runtime/appStorage";
import {configureBrowserNotesDatabase} from "../../core/features/notes/gateways/browserNotesDatabase";
import {createMemoryNotesDatabase} from "../../core/runtime/testing/memoryNotesDatabase";
import {readState, writeState} from "../../core/features/notes/gateways/browserNotesStorage";
import {createBrowserNoteHbsAuthority} from "../../core/features/notes/gateways/browserNoteHbsAuthority";

const f = vi.hoisted(() => ({ load: vi.fn(), get: vi.fn(), outgoing: vi.fn(), error: vi.fn(),
  resetClient: () => {}, hbsRead: vi.fn(), hbsCommit: vi.fn(), readPreservation: vi.fn(), savePreservation: vi.fn() }));
vi.mock("../../core/services/hbsDocumentGateway", async importOriginal => ({
  ...await importOriginal<typeof import("../../core/services/hbsDocumentGateway")>(),
  getHbsDocumentGateway: () => ({read: f.hbsRead, commit: f.hbsCommit,
    readPreservation: f.readPreservation, savePreservation: f.savePreservation}),
}));
vi.mock("../../core/services/notifications", () => ({ notify: { error: f.error, info: vi.fn() } }));
vi.mock("../../core/features/notes/NotesClient", async importOriginal => {
  const actual = await importOriginal<typeof import("../../core/features/notes/NotesClient")>();
  const { browserNotesGateway } = await import("../../core/features/notes/gateways/browserNotesGateway");
  const gateway = { ...browserNotesGateway, loadDocument: f.load, getReadingNote: f.get, listOutgoing: f.outgoing, listBacklinks: async () => [],
    listPages: async () => [], listReadingNotes: async () => [], listNoteTags: async () => [] };
  let client = new actual.NotesClient(gateway);
  f.resetClient = () => { client = new actual.NotesClient(gateway); };
  return { useNotesClient: () => client, useNotesVersion: () => 0, useNoteTagsVersion: () => 0 };
});
vi.mock("../../core/features/notes/agent/useNoteAcceptance", () => ({ useNoteAcceptance: () => ({ state: { phase: "idle" }, registerEditor: vi.fn() }) }));
vi.mock("../../core/features/notes/links/useResearchObjectTargets", () => ({ useResearchObjectTargets: () => [] }));
vi.mock("../../core/features/agent/context/AgentContextProvider", () => ({ useAgentContextCandidate: vi.fn() }));
vi.mock("../../../../../plugins/note_manager/src/features/notes/workspace/useUnifiedNoteIndex", () => ({ useUnifiedNoteIndex: () => ({ items: [], now: new Date(), isLoading: false, hasMore: false, refresh: vi.fn(), loadMore: vi.fn() }) }));
vi.mock("../../../../../plugins/note_manager/src/features/notes/workspace/NotesExplorer", () => ({ NotesExplorer: () => null }));
vi.mock("../../core/features/notes/workspace/NoteDocumentView", () => ({ NoteDocumentView: ({ snapshot }: { snapshot: NoteDocumentSnapshot }) => <div data-testid="note-document">{snapshot.page.title}</div> }));

let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
const saved: NoteDocumentSnapshot = {
  page: { id: notePageId("statement-note"), schemaVersion: 1, title: "语句正文", revision: 1, position: 0,
    status: "active", isFavorite: false, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" },
  document: { schemaVersion: 1, pageId: notePageId("statement-note"), blocks: [] },
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
async function open() {
  await act(async () => { window.dispatchEvent(new CustomEvent("research-assistant:open-annotation-note", {
    detail: { pageId: saved.page.id, kind: "statement" },
  })); });
}
beforeEach(async () => {
  vi.resetAllMocks(); f.resetClient();
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value), removeItem: (key: string) => values.delete(key),
    clear: () => values.clear(), key: (index: number) => [...values.keys()][index] ?? null, get length() { return values.size; } });
  selectAppStorage(localStorage); configureBrowserNotesDatabase(createMemoryNotesDatabase().open);
  const state = await readState();
  await writeState({...state, pages: [saved.page], documents: {[saved.page.id]: saved.document}});
  const authority = createBrowserNoteHbsAuthority();
  f.hbsRead.mockImplementation(async (_kind, id) => authority.read(id));
  f.hbsCommit.mockImplementation(async (_kind, id, request) => authority.commit(id, request));
  f.readPreservation.mockImplementation(async (_kind, id) => authority.readPreservation(id));
  f.savePreservation.mockImplementation(async (_kind, id, checkpoint) => authority.savePreservation(id, checkpoint));
  f.load.mockResolvedValue(saved); f.outgoing.mockResolvedValue([]);
  f.get.mockRejectedValue(new NotesGatewayError("not_found","reading source missing"));
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); selectAppStorage(window.localStorage); });

it("never renders the welcome screen while opening and reopening a statement note, and reuses the loaded document", async () => {
  const pending = deferred<[]>(); f.outgoing.mockReturnValue(pending.promise);
  const welcomeFrames: string[] = [];
  const observer = new MutationObserver(() => {
    if (document.body.textContent?.includes("建立你的研究笔记")) welcomeFrames.push(document.body.textContent);
  });
  observer.observe(document.body, { subtree: true, childList: true });
  try {
    await act(async () => root.render(<StrictMode><ReadingNoteFloatingPanel /></StrictMode>));
    await open();
    expect(document.querySelector('[aria-label="语句笔记编辑面板"]')).not.toBeNull();
    expect(document.querySelector('main[aria-busy="true"]')).not.toBeNull();
    expect(document.body.textContent).not.toMatch(/建立你的研究笔记|新建页面/);
    expect(f.load).toHaveBeenCalledOnce();
    await act(async () => pending.resolve([]));
    expect(document.querySelector('[data-testid="note-document"]')?.textContent).toBe("语句正文");
    expect(useAgentContextCandidate).toHaveBeenCalledWith(expect.objectContaining({resource:{kind:"note",resourceId:saved.page.id}}),true,"floating");
    expect(document.querySelector('main[aria-busy="true"]')).toBeNull();
    await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="关闭阅读笔记"]')!.click());
    await open();
    expect(document.querySelector('[data-testid="note-document"]')).not.toBeNull();
    expect(f.load).toHaveBeenCalledTimes(2);
    expect(welcomeFrames).toEqual([]);
  } finally { observer.disconnect(); }
});

it("reports failed loading through notifications and retries without showing new-page controls", async () => {
  f.outgoing.mockRejectedValueOnce(new Error("关联加载失败"));
  await act(async () => root.render(<ReadingNoteFloatingPanel />));
  await open();
  expect(f.error).toHaveBeenCalledWith("关联加载失败");
  expect(document.body.textContent).not.toMatch(/建立你的研究笔记|新建页面/);
  const retry = [...document.querySelectorAll("button")].find(button => button.textContent === "重新加载笔记");
  expect(retry).toBeDefined();
  await act(async () => retry!.click());
  expect(document.querySelector('[data-testid="note-document"]')).not.toBeNull();
  expect(f.load).toHaveBeenCalledTimes(2);
});

it("shows a missing-note page and diagnostics instead of a loading title, and returns home", async () => {
  f.load.mockRejectedValueOnce(new NotesGatewayError("not_found", "operation=open-missing · 找不到笔记页面"));
  const home = vi.fn();
  function Harness() {
    const [id, setId] = useState<typeof saved.page.id | undefined>(saved.page.id);
    return <NotesWorkspace {...(id ? {initialPageId: id} : {})} onHome={() => { home(); setId(undefined); }} />;
  }
  await act(async () => root.render(<DeveloperModeProvider><DeveloperModeToggle /><Harness /></DeveloperModeProvider>));
  expect(document.querySelector(".notes-document-pending h1")?.textContent).toBe("Oops，找不到这篇笔记");
  expect(document.querySelector(".notes-document-pending")?.textContent).not.toContain("正在打开笔记");
  expect(document.querySelector(".notes-document-diagnostics")).toBeNull();
  await act(async () => document.querySelector<HTMLInputElement>('.developer-mode-toggle input')!.click());
  expect(document.querySelector(".notes-document-diagnostics pre")?.textContent).toContain("not_found");
  expect(document.querySelector(".notes-document-diagnostics pre")?.textContent).toContain("operation=open-missing");
  expect(document.querySelector(".notes-document-diagnostics pre")?.textContent).toContain(saved.page.id);
  await act(async () => document.querySelector<HTMLInputElement>('.developer-mode-toggle input')!.click());
  expect(document.querySelector(".notes-document-diagnostics")).toBeNull();
  const back = [...document.querySelectorAll("button")].find(button => button.textContent === "返回笔记");
  await act(async () => back!.click());
  expect(home).toHaveBeenCalledOnce();
  expect(document.querySelector(".notes-document-pending")).toBeNull();
  expect(document.body.textContent).toContain("建立你的研究笔记");
});

it("still shows the welcome screen in an ordinary workspace with no requested note", async () => {
  await act(async () => root.render(<NotesWorkspace />));
  expect(document.body.textContent).toContain("建立你的研究笔记");
  expect(document.body.textContent).toContain("新建页面");
  expect(f.load).not.toHaveBeenCalled();
});

it("publishes an ordinary note tab with tab priority",async()=>{
  await act(async()=>root.render(<NotesWorkspace initialPageId={saved.page.id}/>));
  expect(useAgentContextCandidate).toHaveBeenCalledWith(expect.objectContaining({resource:{kind:"note",resourceId:saved.page.id}}),true,"tab");
});

it.each([true, false])("recommends the owning paper for a literature note (floating: %s)",async embedded=>{
  const linked = {...saved, page: {...saved.page, literaturePaperId: "paper"}};
  f.load.mockResolvedValue(linked);
  await act(async()=>root.render(<NotesWorkspace initialPageId={saved.page.id} embedded={embedded}/>));
  expect(useAgentContextCandidate).toHaveBeenLastCalledWith({resource:{kind:"paper",resourceId:"paper"},label:"文献"},true,embedded ? "floating" : "tab");
});

it("recommends the source paper for a floating PDF annotation",async()=>{
  const reading:ReadingNoteBlock={id:noteBlockId("annotation"),pageId:saved.page.id,noteType:"annotation",body:"",color:"green",
    status:"active",isFavorite:false,revision:1,projectIds:[],tagIds:[],createdAt:"now",updatedAt:"now",
    source:{paperId:"paper",paperTitle:"Source paper",attachmentId:"pdf",contentSha256:"",pageIndex:0,
      rects:[],quoteExact:"",quotePrefix:"",quoteSuffix:"",state:"exact"}};
  f.get.mockResolvedValue(reading);
  await act(async()=>root.render(<NotesWorkspace initialPageId={saved.page.id} readingSource={reading} embedded/>));
  expect(useAgentContextCandidate).toHaveBeenLastCalledWith({resource:{kind:"paper",resourceId:"paper"},label:"Source paper"},true,"floating");
});

it("hides the navigation opener while its drawer is open and restores it after dismissal", async () => {
  await act(async () => root.render(<NotesWorkspace />));
  const opener = () => document.querySelector<HTMLButtonElement>('[aria-label="打开笔记导航"]');
  expect(opener()).not.toBeNull();
  await act(async () => opener()!.click());
  expect(opener()).toBeNull();
  await act(async () => document.querySelector<HTMLButtonElement>('.notes-explorer-backdrop')!.click());
  expect(opener()).not.toBeNull();
  expect(document.activeElement).toBe(opener());
});

it("only intercepts reading-note opens while the standalone workspace is active", async () => {
  const floatingOpen = vi.fn();
  const id = noteBlockId("reading-source");
  f.get.mockResolvedValue({ id, pageId: saved.page.id,revision:1,body:"阅读正文",source:{paperTitle:"合成文献",quoteExact:"摘录"} });
  window.addEventListener("research-assistant:open-reading-note", floatingOpen);
  const request = () => window.dispatchEvent(new CustomEvent("research-assistant:open-reading-note", { detail: id }));
  try {
    await act(async () => root.render(<NotesWorkspace active={false} />));
    await act(async () => { request(); });
    expect(floatingOpen).toHaveBeenCalledOnce();
    expect(f.get).not.toHaveBeenCalled();
    await act(async () => root.render(<NotesWorkspace active />));
    await act(async () => { request(); });
    expect(floatingOpen).toHaveBeenCalledOnce();
    expect(f.get).toHaveBeenCalledWith(id);
    expect(document.querySelector('[data-testid="note-document"]')).not.toBeNull();
    await act(async () => root.render(<NotesWorkspace active={false} />));
    await act(async () => { request(); });
    expect(floatingOpen).toHaveBeenCalledTimes(2);
    expect(f.get).toHaveBeenCalledOnce();
  } finally { window.removeEventListener("research-assistant:open-reading-note", floatingOpen); }
});
