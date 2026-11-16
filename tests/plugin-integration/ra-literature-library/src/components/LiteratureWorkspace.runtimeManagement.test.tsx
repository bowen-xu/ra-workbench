// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import type {DuplicateCandidate, LiteratureItem, MergeOperation} from "@research-assistant/core/literature";
import {emptyNalState} from "@research-assistant/core/nal";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {notify} from "@research-assistant/workbench-sdk/feedback";
import {createAppRuntime, getAppRuntime, selectAppRuntime, NotesProvider} from "../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {BuiltinPluginConfigurationTest} from "../../../../../../integration/web/src/app/builtinPluginConfiguration.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getLiteratureGateway} from "../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import {getReferenceRecordGateway} from "../../../../../../plugins/ra-literature-library/src/services/referenceRecordGateway";
import {LIBRARY_SESSION_KEY} from "../../../../../../plugins/ra-literature-library/src/features/reader/session/librarySessionStorage";
import {readerSessionId} from "../../../../../../plugins/ra-literature-library/src/features/reader/session/ReaderSessionRegistry";
import {literatureLibraryManifest} from "../../../../../../plugins/ra-literature-library/src/public/manifest";
import {LiteratureWorkspace} from "../../../../../../plugins/ra-literature-library/src/components/LiteratureWorkspace";

const effects = vi.hoisted(() => ({confirm: vi.fn(async (_message: string) => true)}));
vi.mock("@research-assistant/workbench-sdk/platform", async importOriginal => ({
  ...await importOriginal<typeof import("@research-assistant/workbench-sdk/platform")>(), confirm: effects.confirm,
}));
// Reader and Overview are the original components. This management fixture
// rejects unrelated L03/domain calls; it does not claim PDF or HBS saves.

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();
  get length() {return this.values.size;}
  getItem(key: string) {return this.values.get(key) ?? null;}
  setItem(key: string, value: string) {this.values.set(key, value);}
  removeItem(key: string) {this.values.delete(key);}
  key(index: number) {return [...this.values.keys()][index] ?? null;}
  clear() {this.values.clear();}
}
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {resolve = yes; reject = no;});
  return {promise, resolve, reject};
}
function paper(id: string, title: string): LiteratureItem {
  return {id, itemType: "journalArticle", title, abstractText: "", doi: "", containerTitle: "",
    volume: "", issue: "", pages: "", url: "", language: "", bibtexKey: "", creators: [], tags: [],
    projectIds: ["synthetic-project"], metadataSources: [], readingStatus: "unread", createdAt: "2026", updatedAt: "2026",
    attachments: id === "left" ? [{id: "left-pdf", displayName: "synthetic.pdf", mimeType: "application/pdf",
      byteSize: 24, contentSha256: "a".repeat(64)}] : []};
}

afterEach(() => {
  getAppRuntime().dispose();
  vi.useRealTimers(); vi.restoreAllMocks(); effects.confirm.mockReset(); effects.confirm.mockResolvedValue(true);
  window.localStorage.clear();
  selectAppRuntime({mode: "browser", storage: window.localStorage, gateways: {}, dispose() {}});
  vi.unstubAllGlobals();
});

async function mount(reader = false) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("localStorage", new MemoryStorage());
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL) => {
    const url = new URL(input instanceof Request ? input.url : String(input), window.location.href);
    const sample = new URL("/preview/sample.pdf", window.location.href);
    if (url.href !== sample.href) throw new Error(`Unexpected fixture fetch: ${url.origin}${url.pathname}`);
    // Only the existing preview initializer consumes these synthetic bytes;
    // this management port deliberately rejects unrelated L03 reads.
    return new Response("%PDF-1.4\nsynthetic sample");
  }));
  const runtime = await createAppRuntime("?preview=app", window.localStorage);
  runtime.preview!.configure({delayMs: 0, nextFailure: null});
  const references = getReferenceRecordGateway();
  const records = [paper("left", "Synthetic left"), paper("right", "Synthetic right")];
  const active = new Set(records.map(item => item.id)), trashed = new Set<string>();
  const candidate: DuplicateCandidate = {id: "candidate", leftItemId: "left", leftTitle: records[0]!.title,
    rightItemId: "right", rightTitle: records[1]!.title, reason: "doi", confidence: 0.72};
  let duplicates: readonly DuplicateCandidate[] = [candidate], history: readonly MergeOperation[] = [];
  const unexpected = async () => {throw new Error("Unexpected unrelated runtime method");};
  const trash = (ids: readonly string[]) => {for (const id of ids) {active.delete(id); trashed.add(id);}};
  const owner = {literature: {
    importPdfs: vi.fn(unexpected), attachPdf: vi.fn(unexpected), renameAttachment: vi.fn(unexpected),
    removeAttachment: vi.fn(unexpected), readAttachment: vi.fn(unexpected), revealAttachments: vi.fn(unexpected),
    saveReadingState: vi.fn(unexpected), getReadingState: vi.fn(unexpected),
    isAttachmentIndexed: vi.fn(unexpected), indexAttachment: vi.fn(unexpected),
    list: vi.fn(async (_query?: string) => records.filter(item => active.has(item.id))),
    listTrash: vi.fn(async (_query?: string) => records.filter(item => trashed.has(item.id))
      .map(item => ({...item, trashedAt: "2026-11-06T00:00:00.000Z"}))),
    get: vi.fn(async (id: string) => records.find(item => item.id === id)!),
    listDuplicates: vi.fn(async () => duplicates), listMergeOperations: vi.fn(async () => history),
    decideDuplicate: vi.fn(async (_id: string, _decision: "keep_both" | "dismiss") => {duplicates = [];}),
    mergeDuplicate: vi.fn(async (_id: string, survivorItemId: string) => {
      const mergedItemId = survivorItemId === "left" ? "right" : "left";
      const operation: MergeOperation = {id: "operation", survivorItemId, mergedItemId,
        survivorTitle: records.find(item => item.id === survivorItemId)!.title,
        mergedTitle: records.find(item => item.id === mergedItemId)!.title, state: "applied", createdAt: "2026"};
      active.delete(mergedItemId); duplicates = []; history = [operation]; return operation;
    }),
    undoMerge: vi.fn(async (_id: string) => {for (const item of records) active.add(item.id); duplicates = [candidate]; history = [];}),
    archive: vi.fn(async (id: string) => {active.delete(id);}),
    restore: vi.fn(async (id: string) => {active.add(id);}),
    trash: vi.fn(async (id: string) => {trash([id]);}), trashMany: vi.fn(async (ids: readonly string[]) => {trash(ids);}),
    restoreTrash: vi.fn(async (id: string) => {trashed.delete(id); active.add(id);}),
    save: vi.fn(unexpected), updateMetadataField: vi.fn(unexpected), confirmMetadataFields: vi.fn(unexpected),
    importBibtex: vi.fn(unexpected), previewDoi: vi.fn(unexpected), applyMetadataCandidate: vi.fn(unexpected),
    applyMetadataCandidateFields: vi.fn(unexpected), rejectMetadataCandidate: vi.fn(unexpected),
    getImportJob: vi.fn(unexpected), cancelImportJob: vi.fn(unexpected),
  }, referenceRecords: references, nal: {read: vi.fn(async () => emptyNalState()), mutate: unexpected, ensureAnnotation: unexpected},
  knowledge: {searchConcepts: unexpected, saveConcept: unexpected, listReadingUnits: unexpected, saveReadingUnit: unexpected}, queries: {
    read: vi.fn(async () => ({revision: 1, definitions: [], cache: null, legacyImported: true})),
    save: vi.fn(async () => ({revision: 2, definitions: [], cache: null, legacyImported: true})),
  }} satisfies RuntimeLiteraturePort;
  const release = configureLiteratureServices(owner);
  expect(getLiteratureGateway().trash).not.toBeUndefined();
  if (reader) runtime.storage.setItem(LIBRARY_SESSION_KEY, JSON.stringify({version: 1,
    activeTabId: readerSessionId("left-pdf"), tabs: [{itemId: "left", attachmentId: "left-pdf", contentSha256: "a".repeat(64)}]}));
  const success = vi.spyOn(notify, "success").mockReturnValue("synthetic-success");
  const failure = vi.spyOn(notify, "error").mockReturnValue("synthetic-error");
  vi.spyOn(notify, "info").mockReturnValue("synthetic-info");
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(<NotesProvider><BuiltinPluginConfigurationTest owner="literature_library">
    <LiteratureWorkspace project={undefined} openSource={undefined} onSourceOpened={() => undefined}
      openItemId={undefined} onItemOpened={() => undefined} />
  </BuiltinPluginConfigurationTest></NotesProvider>));
  return {host, owner, success, failure, release, trash,
    row: (id: string) => host.querySelector<HTMLElement>(`.literature-row[data-item-id="${id}"]`)!,
    async close() {await act(async () => root.unmount()); host.remove(); release();},
  };
}
type Harness = Awaited<ReturnType<typeof mount>>;
async function click(button: HTMLElement) {expect(button).not.toBeNull(); await act(async () => button.click());}
function button(root: ParentNode, text: string) {
  return [...root.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === text)!;
}
async function menu(h: Harness, id: string, text: string) {
  await act(async () => h.row(id).dispatchEvent(new MouseEvent("contextmenu", {bubbles: true, clientX: 40, clientY: 40})));
  await click(button(document.querySelector('[aria-label="文献操作"]')!, text));
}
async function undoNotice(h: Harness, label: string) {
  const action = h.success.mock.calls.at(-1)![1]!.action as {label: string; onClick: () => void};
  expect(action.label).toBe(label); await act(async () => action.onClick());
}
async function filterTrash(h: Harness, trash: boolean) {
  vi.useFakeTimers();
  try {
    await click(h.host.querySelector<HTMLElement>(`[aria-label="${trash ? "回收站" : "返回文献库"}"]`)!);
    await act(async () => {
      // Advance the original query/filter debounce, not a guessed IO delay.
      await vi.advanceTimersByTimeAsync(180);
    });
  } finally {vi.useRealTimers();}
}

it.each(["keep_both", "dismiss"] as const)("reviews %s through the configured owner without deleting either paper", async decision => {
  const h = await mount();
  try {
    const history = h.owner.literature.listMergeOperations.mock.calls.length;
    await click(button(h.host, decision === "keep_both" ? "保留两条" : "忽略提示"));
    expect(h.owner.literature.decideDuplicate).toHaveBeenCalledExactlyOnceWith("candidate", decision);
    expect(h.host.querySelector(".duplicate-panel")).toBeNull();
    expect(h.host.querySelectorAll(".literature-row")).toHaveLength(2);
    expect(h.owner.literature.listMergeOperations).toHaveBeenCalledTimes(history);
    expect(h.owner.literature.mergeDuplicate).not.toHaveBeenCalled();
    expect(effects.confirm).not.toHaveBeenCalled();
  } finally {await h.close();}
});

it.each(["left", "right"] as const)("confirms merge to %s and restores via the original undo entry", async survivor => {
  const h = await mount();
  try {
    const pending = deferred<boolean>(); effects.confirm.mockReturnValueOnce(pending.promise);
    const merge = button(h.host, survivor === "left" ? "合并到左侧" : "合并到右侧");
    await click(merge); expect(h.owner.literature.mergeDuplicate).not.toHaveBeenCalled();
    await act(async () => pending.resolve(false));
    expect(h.owner.literature.mergeDuplicate).not.toHaveBeenCalled();
    const duplicates = h.owner.literature.listDuplicates.mock.calls.length;
    const history = h.owner.literature.listMergeOperations.mock.calls.length;
    await click(merge);
    expect(h.owner.literature.mergeDuplicate).toHaveBeenCalledExactlyOnceWith("candidate", survivor);
    expect(h.owner.literature.listDuplicates).toHaveBeenCalledTimes(duplicates + 1);
    expect(h.owner.literature.listMergeOperations).toHaveBeenCalledTimes(history + 1);
    expect(h.row(survivor).classList.contains("current")).toBe(true);
    expect(h.host.querySelectorAll(".literature-row")).toHaveLength(1);
    expect(h.host.querySelector(".merge-history")?.textContent).toContain("1 次合并可撤销");
    if (survivor === "left") await undoNotice(h, "撤销合并");
    else await click(button(h.host.querySelector(".merge-history")!, "撤销合并"));
    expect(h.owner.literature.undoMerge).toHaveBeenCalledExactlyOnceWith("operation");
    expect(h.host.querySelectorAll(".literature-row")).toHaveLength(2);
    expect(h.row(survivor === "left" ? "right" : "left").classList.contains("current")).toBe(true);
    expect(h.host.querySelector(".merge-history")).toBeNull();
  } finally {await h.close();}
});

it("archives only after confirmation and restores through the production notification callback", async () => {
  const h = await mount();
  try {
    const pending = deferred<boolean>(); effects.confirm.mockReturnValueOnce(pending.promise);
    await menu(h, "left", "归档文献…");
    expect(h.owner.literature.archive).not.toHaveBeenCalled();
    await act(async () => pending.resolve(true));
    expect(h.owner.literature.archive).toHaveBeenCalledExactlyOnceWith("left");
    expect(h.row("left")).toBeNull();
    await undoNotice(h, "撤销归档");
    expect(h.owner.literature.restore).toHaveBeenCalledExactlyOnceWith("left");
    expect(h.row("left")).not.toBeNull();
  } finally {await h.close();}
});

it("trashes a selected paper, closes its retained reader tab, then restores from the original trash view", async () => {
  const h = await mount(true);
  try {
    expect(h.host.querySelectorAll('[aria-label="文献库标签页"] [role="tab"]')).toHaveLength(1);
    await click(h.host.querySelector<HTMLElement>('[aria-label="返回文献列表"]')!);
    const pending = deferred<boolean>(); effects.confirm.mockReturnValueOnce(pending.promise);
    await menu(h, "left", "移入回收站…");
    expect(h.owner.literature.trash).not.toHaveBeenCalled();
    expect(h.host.querySelectorAll('[aria-label="文献库标签页"] [role="tab"]')).toHaveLength(1);
    await act(async () => pending.resolve(true));
    expect(h.owner.literature.trash).toHaveBeenCalledExactlyOnceWith("left");
    expect(h.row("left")).toBeNull(); expect(h.host.querySelectorAll('[aria-label="文献库标签页"] [role="tab"]')).toHaveLength(0);
    await filterTrash(h, true);
    expect(h.owner.literature.listTrash).toHaveBeenCalledWith("");
    expect(h.row("left")).not.toBeNull();
    await menu(h, "left", "从回收站恢复");
    expect(h.owner.literature.restoreTrash).toHaveBeenCalledExactlyOnceWith("left");
    expect(h.row("left")).toBeNull();
    await filterTrash(h, false);
    expect(h.row("left")).not.toBeNull(); expect(h.host.querySelectorAll('[aria-label="文献库标签页"] [role="tab"]')).toHaveLength(0);
  } finally {await h.close();}
});

it.each(["merge", "archive", "trash"] as const)("cancelled or failed %s confirmation preserves both rows and dispatches no write", async kind => {
  const h = await mount();
  try {
    const trigger = () => kind === "merge" ? click(button(h.host, "合并到左侧"))
      : menu(h, "left", kind === "archive" ? "归档文献…" : "移入回收站…");
    const cancelled = deferred<boolean>(); effects.confirm.mockReturnValueOnce(cancelled.promise);
    await trigger();
    expect(h.owner.literature.mergeDuplicate).not.toHaveBeenCalled();
    expect(h.owner.literature.archive).not.toHaveBeenCalled(); expect(h.owner.literature.trash).not.toHaveBeenCalled();
    await act(async () => cancelled.resolve(false));
    expect(h.owner.literature.mergeDuplicate).not.toHaveBeenCalled();
    expect(h.owner.literature.archive).not.toHaveBeenCalled(); expect(h.owner.literature.trash).not.toHaveBeenCalled();
    const denied = deferred<boolean>(); effects.confirm.mockReturnValueOnce(denied.promise);
    await trigger();
    await act(async () => denied.reject(new Error("PLATFORM_CONFIRMATION_FAILED: synthetic denied confirmation")));
    expect(effects.confirm).toHaveBeenCalledTimes(2);
    expect(h.owner.literature.mergeDuplicate).not.toHaveBeenCalled();
    expect(h.owner.literature.archive).not.toHaveBeenCalled(); expect(h.owner.literature.trash).not.toHaveBeenCalled();
    expect(h.host.querySelectorAll(".literature-row")).toHaveLength(2);
    expect(h.failure).toHaveBeenCalledWith(expect.stringContaining("PLATFORM_CONFIRMATION_FAILED"));
  } finally {await h.close();}
});

it("keeps the captured batch and keyboard lock across asynchronous confirmation and the original trash operation", async () => {
  const h = await mount();
  const key = async (repeat = false) => act(async () => h.row("left").dispatchEvent(
    new KeyboardEvent("keydown", {key: "Delete", repeat, bubbles: true, cancelable: true})));
  try {
    await click(h.row("left"));
    await act(async () => h.row("right").dispatchEvent(new MouseEvent("click", {shiftKey: true, bubbles: true})));
    expect(h.host.querySelectorAll(".literature-row.selected")).toHaveLength(2);
    const captured = [...h.host.querySelectorAll<HTMLElement>(".literature-row.selected")].map(row => row.dataset.itemId!);
    const cancelled = deferred<boolean>(); effects.confirm.mockReturnValueOnce(cancelled.promise);
    await key(); await key(); await key(true);
    expect(effects.confirm).toHaveBeenCalledOnce(); expect(h.owner.literature.trashMany).not.toHaveBeenCalled();
    await act(async () => cancelled.resolve(false));
    expect(h.owner.literature.trashMany).not.toHaveBeenCalled();
    const denied = deferred<boolean>(); effects.confirm.mockReturnValueOnce(denied.promise);
    await key(); await act(async () => denied.reject(new Error("Synthetic batch confirmation denied")));
    expect(h.failure).toHaveBeenCalledWith("Synthetic batch confirmation denied");
    expect(h.owner.literature.trashMany).not.toHaveBeenCalled();
    const confirmed = deferred<boolean>(), write = deferred<void>(); effects.confirm.mockReturnValueOnce(confirmed.promise);
    h.owner.literature.trashMany.mockImplementationOnce(async ids => {await write.promise; h.trash(ids);});
    await key();
    // A later selection does not retarget an already requested confirmation.
    await click(h.row("right"));
    await act(async () => confirmed.resolve(true)); await key();
    expect(h.owner.literature.trashMany).toHaveBeenCalledExactlyOnceWith(captured);
    expect(effects.confirm).toHaveBeenCalledTimes(3);
    expect(h.host.querySelectorAll(".literature-row")).toHaveLength(2);
    await act(async () => write.resolve(undefined));
    expect(h.host.querySelectorAll(".literature-row")).toHaveLength(0);
    expect(h.owner.literature.trash).not.toHaveBeenCalled();
  } finally {await h.close();}
});

it("does not replay a rejected merge or alter the visible data while reporting its original error", async () => {
  const h = await mount();
  try {
    h.owner.literature.mergeDuplicate.mockRejectedValueOnce(new Error("Synthetic unknown merge; do not replay"));
    await click(button(h.host, "合并到左侧"));
    expect(h.owner.literature.mergeDuplicate).toHaveBeenCalledExactlyOnceWith("candidate", "left");
    expect(h.failure).toHaveBeenCalledWith("Synthetic unknown merge; do not replay");
    expect(h.host.querySelectorAll(".literature-row")).toHaveLength(2);
    expect(h.host.querySelector(".merge-history")).toBeNull();
    expect(literatureLibraryManifest.consumes.required).toContainEqual({id: "core.platform.confirmation", version: 1, provider: "core"});
  } finally {await h.close();}
});
