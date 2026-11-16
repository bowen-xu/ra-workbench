// @vitest-environment happy-dom
import { act, createRef, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, noteRevisionId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import { NotesWorkspace as ProductionNotesWorkspace } from "../../../../../../../../plugins/ra-note-manager/src/features/notes/NotesWorkspace";
import {BuiltinPluginConfigurationTest} from "../../../../../../../../integration/web/src/app/builtinPluginConfiguration.testSupport.tsx";
import { browserNotesGateway } from "../../../../../../../apps/web/src/core/features/notes/gateways/browserNotesGateway";
import { selectAppStorage } from "../../../../../../../apps/web/src/core/runtime/appStorage";
import {CoreNoteWorkspace, CoreNoteWorkspaceHost, isTextEditorHeld, type NoteDocumentView} from "@research-assistant/workbench-sdk/note-documents";
import type {WorkspaceClosePort} from "@research-assistant/workbench-sdk/workbench";
import {notify} from "@research-assistant/workbench-sdk/feedback";

type DocumentProps = ComponentProps<typeof NoteDocumentView>;

function NotesWorkspace(props: ComponentProps<typeof ProductionNotesWorkspace>) {
  return <BuiltinPluginConfigurationTest owner="note_manager"><ProductionNotesWorkspace {...props}/></BuiltinPluginConfigurationTest>;
}

const state = vi.hoisted(() => ({ prepare: vi.fn(async () => true), loadMore: vi.fn(), version: 0,
  createPage: vi.fn<NotesGateway["createPage"]>(), updatePage: vi.fn<NotesGateway["updatePage"]>(),
  duplicate: vi.fn<NotesGateway["duplicatePage"]>(), restore: vi.fn<NotesGateway["restoreRevision"]>(),
  history: vi.fn<NotesGateway["listRevisions"]>(async () => []),
  reading: vi.fn<NotesGateway["getReadingNote"]>(async () => {throw new NotesGatewayError("not_found","reading source missing");}), updateReading: vi.fn<NotesGateway["updateReadingNote"]>(),
  importMarkdown: vi.fn<() => Promise<NoteDocumentSnapshot | undefined>>(),
  documentProps: undefined as DocumentProps | undefined,
  openOwner: vi.fn(async (_id: string) => {}), indexState: vi.fn(async () => "dirty"), rebuild: vi.fn(async () => {}) }));
const items = Array.from({length: 50}, (_, id) => ({id: String(id), title: `笔记 ${id}`, snippet: "", category: "independent" as const,
  updatedAt: "2026-10-22T10:00:00Z", status: "active" as const, isFavorite: false, target: {kind: "page" as const, pageId: notePageId(String(id))}}));
vi.mock("../../../../../../../apps/web/src/core/services/notifications", () => ({notify: {error: vi.fn(), info: vi.fn()}}));
vi.mock("../../../../../../../apps/web/src/core/features/notes/NotesClient", async () => {
  const {NotePages}=await import("../../../../../../../apps/web/src/core/features/notes/workspace/NotePages");
  const {NoteCommandHistory}=await import("../../../../../../../apps/web/src/core/features/notes/editor/NoteCommandHistory");
  const gateway = {createPage: state.createPage, updatePage: state.updatePage, loadDocument: async (id: string) => /^\d+$/.test(id) ? ({
    page: {id: notePageId(id), title: `笔记 ${id}`, schemaVersion: 1, revision: 1, position: 0, status: "active", isFavorite: false, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now"},
    document: {schemaVersion: 1, pageId: notePageId(id), blocks: []},
  }) : browserNotesGateway.loadDocument(notePageId(id)), listOutgoing: async () => [], listBacklinks: async () => [], listNoteTags: async () => [],
  getReadingNote:state.reading, updateReadingNote:state.updateReading, duplicatePage:state.duplicate,
  restoreRevision:state.restore, listRevisions:state.history, getNoteIndexState: state.indexState, rebuildNoteIndex: state.rebuild};
  const client = {gateway, links: gateway, history:new NoteCommandHistory(),notifyChanged: vi.fn(),getChangesSince:()=>[],
    mutate: async <T,>(operation: (port: typeof gateway) => Promise<T>) => operation(gateway)};
  Object.assign(client,{creation:new NotePages(client as unknown as import("../../../../../../../ra-workbench/apps/web/src/core/features/notes/NotesClient").NotesClient)});
  return {useNotesClient: () => client, useNotesVersion: () => state.version, useNoteTagsVersion: () => 0};
});
vi.mock("../../../../../../../apps/web/src/core/features/notes/workspace/useNoteDocumentSession", async original => {
  const module = await original<typeof import("../../../../../../../ra-workbench/apps/web/src/core/features/notes/workspace/useNoteDocumentSession")>();
  const preserved = new Map<string,unknown>();
  return {...module, useNoteDocumentSession: (options: Parameters<typeof module.useNoteDocumentSession>[0]) => ({...module.useNoteDocumentSession({...options,hbsGateway:{
    read:async(_kind,id)=>{await state.openOwner(id);const loaded=await options.client.gateway.loadDocument(notePageId(id));return{version:loaded.page.revision,saved:loaded.document,baseline:loaded.document};},
    commit:async()=>{throw new Error("unexpected document write");},readPreservation:async(_kind,id)=>preserved.get(id)??null,
    savePreservation:async(_kind,id,value)=>{preserved.set(id,value);return{revision:1};},
  }}), ensureDurable: state.prepare})};
});

it("shows the target title and loading state immediately while the previous editor stays hidden and retained", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<NotesWorkspace />));
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-explorer [aria-label="打开笔记：笔记 0"]')!.click());
  const old = host.querySelector('.retained-note-editor')!;
  let release!: () => void;
  state.openOwner.mockImplementationOnce(() => new Promise<void>(resolve => {release = resolve;}));
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-explorer [aria-label="打开笔记：笔记 1"]')!.click());
  const during = {title: host.querySelector('.notes-document-pending h1')?.textContent,
    status: host.querySelector('[role="status"]')?.textContent, oldHidden: old.hasAttribute("hidden")};
  await act(async () => release());
  expect(during).toEqual({title: "笔记 1", status: "正在打开笔记…", oldHidden: true});
  expect(host.querySelector('.notes-document-pending')).toBeNull();
  expect(host.querySelector('.retained-note-editor:not([hidden])')).not.toBeNull();
});

it("does not open an earlier click after a later navigation has passed its durable guard", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<NotesWorkspace />));
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-explorer [aria-label="打开笔记：笔记 0"]')!.click());
  let release!: (value: boolean) => void;
  state.prepare.mockImplementationOnce(() => new Promise<boolean>(resolve => {release = resolve;}));
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-explorer [aria-label="打开笔记：笔记 1"]')!.click());
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-explorer [aria-label="打开笔记：笔记 2"]')!.click());
  await act(async () => release(true));
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="保留的正文编辑器"]')?.value).toBe("笔记 2");
  expect(host.querySelector('.retained-note-editor:not([hidden])')?.textContent).toContain("笔记 2");
});
vi.mock("../../../../../../../apps/web/src/core/features/notes/agent/useNoteAcceptance", () => ({useNoteAcceptance: () => ({state: {phase: "idle"}, registerEditor: vi.fn()})}));
vi.mock("../../../../../../../apps/web/src/core/features/notes/links/useResearchObjectTargets", () => ({useResearchObjectTargets: () => []}));
vi.mock("../../../../../../../apps/web/src/core/features/agent/context/AgentContextProvider", () => ({useAgentContextCandidate: vi.fn()}));
vi.mock("../../../../../../../../plugins/ra-note-manager/src/features/notes/workspace/useUnifiedNoteIndex", () => ({useUnifiedNoteIndex: () => ({items, libraryItems: items, libraryAllItems: items, libraryHasMore: true, librarySort: {field: "updatedAt", direction: "descending"}, setLibrarySort: vi.fn(), now: new Date("2026-10-22"), isLoading: false,libraryIsLoading:false,warnings:[],libraryWarnings:[],hasMore: true, refresh: vi.fn(),retry:vi.fn(),loadMore: state.loadMore,loadLibraryMore:state.loadMore})}));
vi.mock("../../../../../../../apps/web/src/core/features/notes/workspace/NoteDocumentView", () => ({NoteDocumentView: (props: DocumentProps) => {
  state.documentProps = props;
  return <textarea aria-label="保留的正文编辑器" defaultValue={props.snapshot.page.title} />;
}}));
vi.mock("../../../../../../../../plugins/ra-note-manager/src/features/notes/transfer/noteLibraryImport", () => ({importMarkdownIntoLibrary: () => state.importMarkdown()}));

let root: ReturnType<typeof createRoot>, host: HTMLDivElement;
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.clearAllMocks(); vi.unstubAllGlobals();
  state.documentProps = undefined; state.duplicate.mockReset(); state.restore.mockReset(); state.history.mockReset().mockResolvedValue([]);
  state.importMarkdown.mockReset(); state.reading.mockReset().mockRejectedValue(new NotesGatewayError("not_found","reading source missing")); state.updateReading.mockReset(); state.prepare.mockReset().mockResolvedValue(true);
});

it("keeps note management preferences on their existing Core storage keys across reopening", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear(); selectAppStorage(localStorage);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<NotesWorkspace/>));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>(".notes-scope-tabs button")].find(button => button.textContent === "收藏")?.click());
  expect(localStorage.getItem("research-assistant:workspace.v1:notes-scope")).toBe('"favorites"');
  await act(async () => root.render(<NotesWorkspace key="reopen"/>));
  expect(host.querySelector('.notes-scope-tabs [aria-pressed="true"]')?.textContent).toBe("收藏");
  localStorage.clear();
});

it.each([
  ["recent", "heading"], ["favorites", "heading"],
  ["recent", "empty state"], ["favorites", "empty state"],
] as const)("creates and opens a canonical empty page from the %s %s", async (scope, entry) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  window.localStorage.clear(); selectAppStorage(window.localStorage);
  state.createPage.mockImplementation(draft => browserNotesGateway.createPage(draft));
  state.updatePage.mockImplementation(request => browserNotesGateway.updatePage(request));
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  try {
    await act(async () => root.render(<NotesWorkspace />));
    if (scope === "favorites") await act(async () => {
      [...host.querySelectorAll<HTMLButtonElement>(".notes-scope-tabs button")].find(button => button.textContent === "收藏")!.click();
    });
    const create = host.querySelector<HTMLButtonElement>(entry === "heading"
      ? '.notes-explorer [aria-label="新建独立笔记"]' : '.note-empty-state .primary-button');
    expect(create).not.toBeNull();
    await act(async () => create!.click());
    expect(state.createPage).not.toHaveBeenCalled();
    const pages = await browserNotesGateway.listPages({kind:"all"});
    expect(pages).toHaveLength(1);
    expect(pages[0]?.isFavorite).toBe(scope === "favorites");
    expect(host.querySelector('.notes-scope-tabs [aria-pressed="true"]')?.textContent).toBe(scope === "favorites" ? "收藏" : "近期");
    expect(host.querySelector('.retained-note-editor:not([hidden])')).not.toBeNull();
    expect(state.updatePage).not.toHaveBeenCalled();
  } finally { window.localStorage.clear(); }
});

it("opens View All on the right after the durable guard, keeps the sidebar compact and retains the original editor session", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value), removeItem: (key: string) => values.delete(key)});
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<NotesWorkspace />));
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-explorer [aria-label="打开笔记：笔记 0"]')!.click());
  // The mocked document header may unmount on Home. The actual retained
  // editing host owns document/history and is the lifecycle under test.
  const editor = host.querySelector<HTMLElement>(".retained-note-editor .bn-editor")!;
  const before = editor.textContent;
  state.prepare.mockResolvedValueOnce(false);
  const viewAll = host.querySelector<HTMLButtonElement>(".notes-explorer > .notes-load-more")!;
  await act(async () => viewAll.click());
  expect(host.querySelector(".notes-library")).toBeNull(); expect(host.querySelector(".retained-note-editor .bn-editor")).toBe(editor);
  await act(async () => viewAll.click());
  expect(host.querySelectorAll(".notes-explorer .note-recent-row")).toHaveLength(20);
  expect(host.querySelectorAll(".notes-library tbody tr")).toHaveLength(50);
  expect(editor.closest("main")?.hidden).toBe(true); expect(editor.textContent).toBe(before);
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-library [aria-label="打开笔记：笔记 0"]')!.click());
  expect(host.querySelector(".notes-library")).toBeNull(); expect(editor.closest("main")?.hidden).toBe(false);
  expect(host.querySelector(".retained-note-editor .bn-editor")).toBe(editor); expect(editor.textContent).toBe(before);
  expect(state.prepare).toHaveBeenCalledTimes(4);
  state.version++;
  await act(async () => root.render(<NotesWorkspace active={false} />));
  await act(async () => root.render(<NotesWorkspace active />));
  expect(state.indexState).not.toHaveBeenCalled();
  expect(state.rebuild).not.toHaveBeenCalled();
});

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, refuse) => { resolve = accept; reject = refuse; });
  return {promise, resolve, reject};
}
async function mountManaged(readingSource?: ReadingNoteBlock) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear(); selectAppStorage(localStorage);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const ref = createRef<WorkspaceClosePort>();
  await act(async () => root.render(<CoreNoteWorkspaceHost><NotesWorkspace ref={ref} {...(readingSource ? {readingSource} : {})}/></CoreNoteWorkspaceHost>));
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-explorer [aria-label="打开笔记：笔记 0"]')!.click());
  return ref;
}
function documentProps(): DocumentProps {
  if (!state.documentProps) throw new Error("The real Core document projection has not mounted");
  return state.documentProps;
}
async function reopenOn(id: string) {
  await act(async () => root.render(<CoreNoteWorkspaceHost><NotesWorkspace key="new-instance"/></CoreNoteWorkspaceHost>));
  await act(async () => host.querySelector<HTMLButtonElement>(`.notes-explorer [aria-label="打开笔记：笔记 ${id}"]`)!.click());
}

it("blocks close synchronously through overlapping actions and only releases after both settle", async () => {
  const ref = await mountManaged(), copy = deferred<NoteDocumentSnapshot>(), history = deferred<Awaited<ReturnType<NotesGateway["listRevisions"]>>>();
  const original = documentProps().snapshot;
  state.duplicate.mockReturnValueOnce(copy.promise); state.history.mockReturnValueOnce(history.promise);
  await act(async () => { documentProps().onDuplicate(); documentProps().onOpenHistory(); });
  expect(await ref.current!.prepareClose()).toBe(false);
  expect(state.duplicate).toHaveBeenCalledOnce(); expect(state.history).toHaveBeenCalledOnce();
  await act(async () => history.resolve([]));
  expect(await ref.current!.prepareClose()).toBe(false);
  await act(async () => copy.resolve({...original, page: {...original.page, id: notePageId("1")}, document: {...original.document, pageId: notePageId("1")}}));
  expect(await ref.current!.prepareClose()).toBe(true);
  expect(documentProps().snapshot.page.id).toBe("1");
});

it("does not dispatch creation after the old instance's durable preparation finishes late", async () => {
  const ref = await mountManaged(), saving = deferred<boolean>();
  state.prepare.mockReturnValueOnce(saving.promise);
  await act(async () => host.querySelector<HTMLButtonElement>('.notes-explorer [aria-label="新建独立笔记"]')!.click());
  expect(await ref.current!.prepareClose()).toBe(false);
  await reopenOn("1");
  const preferences = localStorage.getItem("research-assistant:workspace.v1:notes-mode");
  await act(async () => saving.resolve(true));
  expect(await browserNotesGateway.listPages({kind:"all"})).toEqual([]);
  expect(documentProps().snapshot.page.id).toBe("1");
  expect(localStorage.getItem("research-assistant:workspace.v1:notes-mode")).toBe(preferences);
});

it("retains a completed copy receipt but never opens it in the replacement instance", async () => {
  await mountManaged(); const original = documentProps().snapshot, copy = deferred<NoteDocumentSnapshot>();
  state.duplicate.mockReturnValueOnce(copy.promise);
  await act(async () => documentProps().onDuplicate());
  await reopenOn("1"); state.openOwner.mockClear();
  await act(async () => copy.resolve({...original, page: {...original.page, id: notePageId("2")}, document: {...original.document, pageId: notePageId("2")}}));
  expect(state.duplicate).toHaveBeenCalledOnce(); expect(state.openOwner).not.toHaveBeenCalled();
  expect(documentProps().snapshot.page.id).toBe("1");
});

it("does not revive an old presentation continuation after the same mounted view hides and reopens", async () => {
  const ref = await mountManaged(), copy = deferred<NoteDocumentSnapshot>(), original = documentProps().snapshot;
  const editor = host.querySelector(".retained-note-editor .bn-editor");
  state.duplicate.mockReturnValueOnce(copy.promise);
  await act(async () => documentProps().onDuplicate());
  await act(async () => root.render(<CoreNoteWorkspaceHost><NotesWorkspace ref={ref} active={false}/></CoreNoteWorkspaceHost>));
  await act(async () => root.render(<CoreNoteWorkspaceHost><NotesWorkspace ref={ref} active/></CoreNoteWorkspaceHost>));
  state.openOwner.mockClear();
  await act(async () => copy.resolve({...original,page:{...original.page,id:notePageId("2")},document:{...original.document,pageId:notePageId("2")}}));
  expect(documentProps().snapshot.page.id).toBe("0"); expect(state.openOwner).not.toHaveBeenCalled();
  expect(host.querySelector(".retained-note-editor .bn-editor")).toBe(editor);
  expect(await ref.current!.prepareClose()).toBe(true);
});

it("does not let an old home preparation clear the shared Core document after the projection hides", async () => {
  const ref = await mountManaged(), saving = deferred<boolean>();
  state.prepare.mockReturnValueOnce(saving.promise);
  await act(async () => host.querySelector<HTMLButtonElement>(".notes-explorer > .notes-load-more")!.click());
  expect(await ref.current!.prepareClose()).toBe(false);
  await act(async () => root.render(<CoreNoteWorkspaceHost><NotesWorkspace ref={ref} active={false}/></CoreNoteWorkspaceHost>));
  await act(async () => root.render(<CoreNoteWorkspaceHost><NotesWorkspace ref={ref} active/></CoreNoteWorkspaceHost>));
  await act(async () => saving.resolve(true));
  expect(documentProps().snapshot.page.id).toBe("0"); expect(host.querySelector(".notes-library")).toBeNull();
});

it("keeps a fallback Core document when the hidden manager's initial-page preparation completes late", async () => {
  const ref = await mountManaged(), saving = deferred<boolean>();
  state.prepare.mockReturnValueOnce(saving.promise);
  await act(async () => root.render(<CoreNoteWorkspaceHost><NotesWorkspace ref={ref} initialPageId={notePageId("1")} openRequestKey={1}/></CoreNoteWorkspaceHost>));
  expect(await ref.current!.prepareClose()).toBe(false);
  await act(async () => root.render(<CoreNoteWorkspaceHost>
    <NotesWorkspace ref={ref} active={false} initialPageId={notePageId("1")} openRequestKey={1}/>
    <CoreNoteWorkspace embedded={false} active initialPageId={notePageId("2")}/>
  </CoreNoteWorkspaceHost>));
  expect(documentProps().snapshot.page.id).toBe("2");
  const prefix = "research-assistant:workspace.v1:", preferences = ["notes-mode", "notes-scope", "notes-search", "notes-full-index"].map(key => localStorage.getItem(prefix + key));
  state.openOwner.mockClear();
  await act(async () => saving.resolve(true));
  expect(state.openOwner).not.toHaveBeenCalled(); expect(documentProps().snapshot.page.id).toBe("2");
  expect(["notes-mode", "notes-scope", "notes-search", "notes-full-index"].map(key => localStorage.getItem(prefix + key))).toEqual(preferences);
});

it("does not let a completed import change the replacement instance's Core preferences or document", async () => {
  const ref = await mountManaged(), imported = deferred<NoteDocumentSnapshot>();
  const original = documentProps().snapshot;
  state.importMarkdown.mockReturnValueOnce(imported.promise);
  const input = host.querySelector<HTMLInputElement>('input[type="file"]')!;
  Object.defineProperty(input, "files", {value: [new File(["body"], "known.md", {type:"text/markdown"})]});
  await act(async () => input.dispatchEvent(new Event("change", {bubbles:true})));
  expect(await ref.current!.prepareClose()).toBe(false);
  await reopenOn("1");
  const prefix = "research-assistant:workspace.v1:", preferences = ["notes-mode", "notes-scope", "notes-search"].map(key => localStorage.getItem(prefix + key));
  state.openOwner.mockClear();
  await act(async () => imported.resolve(original));
  expect(["notes-mode", "notes-scope", "notes-search"].map(key => localStorage.getItem(prefix + key))).toEqual(preferences);
  expect(state.openOwner).not.toHaveBeenCalled(); expect(documentProps().snapshot.page.id).toBe("1");
});

it("keeps the original close barrier pending until the Markdown confirmation is cancelled", async () => {
  const ref = await mountManaged(), confirmation = deferred<NoteDocumentSnapshot | undefined>();
  const original = documentProps().snapshot;
  state.importMarkdown.mockReturnValueOnce(confirmation.promise);
  const input = host.querySelector<HTMLInputElement>('input[type="file"]')!;
  Object.defineProperty(input, "files", {value: [new File(["body"], "cancel.md", {type: "text/markdown"})]});
  await act(async () => input.dispatchEvent(new Event("change", {bubbles: true})));
  expect(state.importMarkdown).toHaveBeenCalledOnce();
  expect(await ref.current!.prepareClose()).toBe(false);
  await act(async () => confirmation.resolve(undefined));
  expect(await ref.current!.prepareClose()).toBe(true);
  expect(documentProps().snapshot.page.id).toBe(original.page.id);
});

it("settles an admitted restore and releases its original hold without installing into a new owner", async () => {
  const ref = await mountManaged(), original = documentProps().snapshot, restored = deferred<NoteDocumentSnapshot>();
  state.restore.mockReturnValueOnce(restored.promise);
  let operation!: Promise<void>;
  await act(async () => { operation = documentProps().onRestoreRevision(noteRevisionId("known-revision")); });
  expect(state.restore).toHaveBeenCalledOnce(); expect(isTextEditorHeld("0")).toBe(true);
  expect(await ref.current!.prepareClose()).toBe(false);
  await reopenOn("1"); state.openOwner.mockClear();
  await act(async () => { restored.resolve({...original, page:{...original.page,revision:2}}); await operation; });
  expect(isTextEditorHeld("0")).toBe(false); expect(state.openOwner).not.toHaveBeenCalled();
  expect(documentProps().snapshot.page.id).toBe("1");
});

it("reports ordinary copy and history failures through their original safe diagnostics", async () => {
  await mountManaged();
  state.duplicate.mockRejectedValueOnce(new Error("copy_failed: operation fixture-copy"));
  state.history.mockRejectedValueOnce(new Error("history_failed: request fixture-history"));
  await act(async () => { documentProps().onDuplicate(); documentProps().onOpenHistory(); });
  expect(notify.error).toHaveBeenCalledWith("copy_failed: operation fixture-copy");
  expect(notify.error).toHaveBeenCalledWith("history_failed: request fixture-history");
  state.prepare.mockRejectedValueOnce(new Error("open_failed: request fixture-open"));
  await act(async () => documentProps().onOpenPage(notePageId("1")));
  expect(notify.error).toHaveBeenCalledWith("open_failed: request fixture-open");
});

it("does not apply a reading-color revision to a different shared Core document after awaiting its receipt", async () => {
  const reading: ReadingNoteBlock = {id:noteBlockId("reading-0"),pageId:notePageId("0"),noteType:"highlight",body:"Known quote",color:"yellow",status:"active",isFavorite:false,revision:1,projectIds:[],tagIds:[],createdAt:"now",updatedAt:"now",
    source:{paperId:"paper",paperTitle:"Paper",attachmentId:"attachment",contentSha256:"known",pageIndex:0,rects:[],quoteExact:"Known quote",quotePrefix:"",quoteSuffix:"",state:"exact"}};
  const ref = await mountManaged(reading), changed = deferred<ReadingNoteBlock>();
  state.reading.mockResolvedValue(reading); state.updateReading.mockReturnValueOnce(changed.promise);
  await act(async () => documentProps().onReadingColorChange?.("green"));
  expect(state.updateReading).toHaveBeenCalledOnce(); expect(await ref.current!.prepareClose()).toBe(false);
  await act(async () => documentProps().onOpenPage(notePageId("1")));
  await act(async () => changed.resolve({...reading,revision:99,color:"green"}));
  expect(documentProps().snapshot.page.id).toBe("1"); expect(documentProps().snapshot.page.revision).toBe(1);
});
