// @vitest-environment happy-dom
import { act, useCallback, useState, type ComponentProps } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { createAppRuntime } from "../../../../../../integration/web/src/app/runtime/createAppRuntime";
import { getAppRuntime, selectAppRuntime } from "../../../../../apps/web/src/core/runtime/appRuntime";
import { getLiteratureGateway } from "../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import {getNalGateway} from "../../../../../../plugins/ra-literature-library/src/services/nalGateway";
import {getNalQueryGateway} from "../../../../../../plugins/ra-literature-library/src/services/nalQueryGateway";
import {getReferenceRecordGateway} from "../../../../../../plugins/ra-literature-library/src/services/referenceRecordGateway";
import {configureLiteratureServices} from "../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import { LIBRARY_SESSION_KEY } from "../../../../../../plugins/ra-literature-library/src/features/reader/session/librarySessionStorage";
import { LIBRARY_TAB_ID, readerSessionId } from "../../../../../../plugins/ra-literature-library/src/features/reader/session/ReaderSessionRegistry";
import {libraryNavigationRestore} from "../../../../../../plugins/ra-literature-library/src/public/navigation";
import { useWorkspaceNavigation } from "../../../../../apps/web/src/core/useWorkspaceNavigation";
import { LiteratureWorkspace as ProductionLiteratureWorkspace } from "../../../../../../plugins/ra-literature-library/src/components/LiteratureWorkspace";
import {BuiltinPluginConfigurationTest} from "../../../../../../integration/web/src/app/builtinPluginConfiguration.testSupport.tsx";
import { NotesProvider } from "../../../../../apps/web/src/core/features/notes/NotesClient";
import type { ImportJob } from "@research-assistant/core/literature";
import { ApplicationKeyboardPolicy } from "../../../../../apps/web/src/core/runtime/ApplicationKeyboardPolicy";

vi.mock("../../../../../../plugins/ra-literature-library/src/features/reader/PaperReadingReader", () => ({ PaperReadingReader: () => <section>PDF 正文</section> }));
vi.mock("../../../../../../plugins/ra-literature-library/src/features/ai/PaperOverviewPanel", () => ({ PaperOverviewPanel: () => null }));

function LiteratureWorkspace(props: ComponentProps<typeof ProductionLiteratureWorkspace>) {
  return <BuiltinPluginConfigurationTest owner="literature_library"><ProductionLiteratureWorkspace {...props}/></BuiltinPluginConfigurationTest>;
}

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();
  get length() { return this.values.size; }
  getItem(key: string) { return this.values.get(key) ?? null; }
  setItem(key: string, value: string) { this.values.set(key, value); }
  removeItem(key: string) { this.values.delete(key); }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
  clear() { this.values.clear(); }
}

afterEach(() => {
  getAppRuntime().dispose();
  vi.restoreAllMocks();
  window.localStorage.clear();
  selectAppRuntime({ mode: "browser", storage: window.localStorage, gateways: {}, dispose() {} });
  vi.unstubAllGlobals();
});

it("navigates manual import fields under the application Tab policy without submitting", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("localStorage", new MemoryStorage());
  vi.stubGlobal("fetch", vi.fn(async () => new Response("%PDF-1.4\nsynthetic sample")));
  const runtime = await createAppRuntime("?preview=app", window.localStorage);
  runtime.preview!.configure({ delayMs: 0, nextFailure: null });
  const save = vi.spyOn(getLiteratureGateway(), "save");
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<><ApplicationKeyboardPolicy /><NotesProvider><LiteratureWorkspace project={undefined} openSource={undefined}
      onSourceOpened={() => undefined} openItemId={undefined} onItemOpened={() => undefined} /></NotesProvider></>));
    expect(host.querySelector('[aria-label="文献库工具栏"] [title]')).toBeNull();
    const pdf = host.querySelector('[aria-label="从 PDF 添加文献"]');
    expect(pdf?.querySelector('.lucide-chevron-down')).toBeNull();
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="手动添加文献"]')!.click());
    const form = document.querySelector<HTMLFormElement>('#literature-import-popover')!;
    const title = form.querySelector<HTMLInputElement>('[placeholder="标题（必填）"]')!;
    const type = form.querySelector<HTMLSelectElement>('[aria-label="条目类型"]')!;
    const tab = async (target: Element, shiftKey = false) => {
      await act(async () => target.dispatchEvent(new KeyboardEvent("keydown", {key: "Tab", shiftKey, bubbles: true, cancelable: true})));
    };
    await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(title, "Manual keyboard draft"); title.dispatchEvent(new Event("input", {bubbles: true})); });
    await tab(title); expect(document.activeElement).toBe(type);
    await tab(type, true); expect(document.activeElement).toBe(title);
    await act(async () => { type.value = "conferencePaper"; type.dispatchEvent(new Event("change", {bubbles: true})); });
    const container = form.querySelector('[placeholder="来源 / 容器"]')!;
    await tab(container); expect(document.activeElement?.getAttribute('placeholder')).toBe('会议名称');
    await act(async () => { type.value = "journalArticle"; type.dispatchEvent(new Event("change", {bubbles: true})); });
    await tab(container); expect(document.activeElement?.getAttribute('placeholder')).toBe('作者：姓, 名；多位用分号');
    const abstract = form.querySelector('textarea')!;
    await tab(abstract); expect(document.activeElement).toBe(title);
    await tab(title, true); expect(document.activeElement).toBe(abstract);
    expect(title.value).toBe('Manual keyboard draft'); expect(save).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("keeps the empty library message below the table header in the same list viewport", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("localStorage", new MemoryStorage());
  vi.stubGlobal("fetch", vi.fn(async () => new Response("%PDF-1.4\nsynthetic sample")));
  const runtime = await createAppRuntime("?preview=app", window.localStorage);
  runtime.preview!.configure({ delayMs: 0, nextFailure: null });
  vi.spyOn(getLiteratureGateway(), "list").mockResolvedValue([]);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<NotesProvider><LiteratureWorkspace project={undefined} openSource={undefined}
      onSourceOpened={() => undefined} openItemId={undefined} onItemOpened={() => undefined} /></NotesProvider>));
    const viewport = host.querySelector(".literature-table-scroll");
    expect(viewport?.querySelector(".library-empty")?.textContent).toContain("文献库还是空的");
    expect(viewport?.querySelectorAll('[role="columnheader"]')).toHaveLength(5);
    expect(host.querySelector(".literature-list > .library-empty")).toBeNull();
    expect(host.querySelector(".detail-empty")?.textContent).toBe("选择一篇文献查看详情");
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="按标题排序"]')?.click());
    expect(JSON.parse(runtime.storage.getItem("research-assistant:workspace.v1:library-sort") ?? "null")).toEqual({key: "title", direction: "ascending"});
    await act(async () => root.render(<NotesProvider key="reopen"><LiteratureWorkspace project={undefined} openSource={undefined}
      onSourceOpened={() => undefined} openItemId={undefined} onItemOpened={() => undefined} /></NotesProvider>));
    expect(host.querySelector('[role="columnheader"].title')?.getAttribute("aria-sort")).toBe("ascending");
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it.each([false, true])("opens literature details and retains the reader across repeated navigation (pending at startup: %s)", async pendingAtStartup => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("localStorage", new MemoryStorage());
  vi.stubGlobal("fetch", vi.fn(async () => new Response("%PDF-1.4\nsynthetic sample")));
  const runtime = await createAppRuntime("?preview=app", window.localStorage);
  runtime.preview!.configure({ delayMs: 0, nextFailure: null });
  const item = (await getLiteratureGateway().list()).find(candidate => candidate.attachments.length > 0)!;
  const attachment = item.attachments[0]!;
  const tabId = readerSessionId(attachment.id);
  runtime.storage.setItem(LIBRARY_SESSION_KEY, JSON.stringify({ version: 1, activeTabId: tabId,
    tabs: [{ itemId: item.id, attachmentId: attachment.id, contentSha256: attachment.contentSha256 }] }));
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  function Harness() {
    const navigation = useWorkspaceNavigation(undefined, libraryNavigationRestore);
    const [openItemId, setOpenItemId] = useState<string | undefined>(pendingAtStartup ? item.id : undefined);
    const onItemOpened = useCallback(() => setOpenItemId(undefined), []);
    return <>
      <button onClick={() => { navigation.navigate("library"); setOpenItemId(item.id); }}>打开文献来源</button>
      <output>{navigation.location.tabId}</output>
      <LiteratureWorkspace navigationTabId={navigation.location.tabId} onNavigateTab={navigation.visitLibraryTab}
        onSessionsChanged={navigation.reconcileLibrary} project={undefined} openSource={undefined}
        onSourceOpened={() => undefined} openItemId={openItemId} onItemOpened={onItemOpened} />
    </>;
  }
  try {
    await act(async () => root.render(<NotesProvider><Harness /></NotesProvider>));
    const surface = host.querySelector<HTMLElement>(".library-surface")!;
    const readerTab = host.querySelector<HTMLButtonElement>('[role="tab"]')!;
    if (pendingAtStartup) {
      expect(surface.hidden).toBe(false);
      expect(host.querySelector(".literature-row.selected")?.textContent).toContain(item.title);
      await act(async () => readerTab.click());
    }
    expect(surface.hidden).toBe(true);
    expect(readerTab.getAttribute("aria-selected")).toBe("true");
    for (let attempt = 0; attempt < 2; attempt++) {
      const open = [...host.querySelectorAll("button")].find(button => button.textContent === "打开文献来源")!;
      await act(async () => open.click());
      expect(surface.hidden).toBe(false);
      expect(host.querySelector("output")?.textContent).toBe(LIBRARY_TAB_ID);
      expect(host.querySelector(".literature-row.selected")?.textContent).toContain(item.title);
      expect(readerTab.isConnected).toBe(true);
      expect(readerTab.getAttribute("aria-selected")).toBe("false");
      await act(async () => readerTab.click());
      expect(surface.hidden).toBe(true);
    }
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it.each([false, true].flatMap(injected => (["manual", "bibtex", "queued-bibtex"] as const).map(kind => ({kind, injected}))))(
  "selects newly added $kind literature instead of the previous row, including after a filtered refresh (injected owner: $injected)", async ({kind, injected}) => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("localStorage", new MemoryStorage());
  vi.stubGlobal("fetch", vi.fn(async () => new Response("%PDF-1.4\nsynthetic sample")));
  const runtime = await createAppRuntime("?preview=app", window.localStorage);
  runtime.preview!.configure({ delayMs: 0, nextFailure: null });
  const original = getLiteratureGateway(), nal = getNalGateway(), queries = getNalQueryGateway(), references = getReferenceRecordGateway();
  // Only data/side effects are mocked. Both modes mount the original workspace,
  // import actions and metadata dialog; this is not a real Native-host proof.
  const owner: RuntimeLiteraturePort = {literature: {
    importPdfs: vi.fn(original.importPdfs.bind(original)), attachPdf: vi.fn(original.attachPdf.bind(original)),
    renameAttachment: vi.fn(original.renameAttachment.bind(original)), removeAttachment: vi.fn(original.removeAttachment.bind(original)),
    readAttachment: vi.fn(original.readAttachment.bind(original)), revealAttachments: vi.fn(original.revealAttachments.bind(original)),
    saveReadingState: vi.fn(original.saveReadingState.bind(original)), getReadingState: vi.fn(original.getReadingState.bind(original)),
    isAttachmentIndexed: vi.fn(original.isAttachmentIndexed.bind(original)), indexAttachment: vi.fn(original.indexAttachment.bind(original)),
    list: vi.fn(original.list.bind(original)), get: vi.fn(original.get.bind(original)),
    save: vi.fn(original.save.bind(original)), updateMetadataField: vi.fn(original.updateMetadataField.bind(original)),
    confirmMetadataFields: vi.fn(original.confirmMetadataFields.bind(original)), importBibtex: vi.fn(original.importBibtex.bind(original)),
    listDuplicates: vi.fn(original.listDuplicates.bind(original)), listMergeOperations: vi.fn(original.listMergeOperations.bind(original)),
    listTrash: vi.fn(original.listTrash.bind(original)), decideDuplicate: vi.fn(original.decideDuplicate.bind(original)),
    mergeDuplicate: vi.fn(original.mergeDuplicate.bind(original)), undoMerge: vi.fn(original.undoMerge.bind(original)),
    archive: vi.fn(original.archive.bind(original)), restore: vi.fn(original.restore.bind(original)), trash: vi.fn(original.trash.bind(original)),
    trashMany: vi.fn(original.trashMany.bind(original)), restoreTrash: vi.fn(original.restoreTrash.bind(original)),
    previewDoi: vi.fn(original.previewDoi.bind(original)), applyMetadataCandidate: vi.fn(original.applyMetadataCandidate.bind(original)),
    applyMetadataCandidateFields: vi.fn(original.applyMetadataCandidateFields.bind(original)), rejectMetadataCandidate: vi.fn(original.rejectMetadataCandidate.bind(original)),
    getImportJob: vi.fn(original.getImportJob.bind(original)), cancelImportJob: vi.fn(original.cancelImportJob.bind(original)),
  }, referenceRecords: references, nal: {read: nal.read.bind(nal), mutate: nal.mutate.bind(nal), ensureAnnotation: nal.ensureAnnotation.bind(nal)},
    knowledge: {searchConcepts: async () => {throw new Error("Unrelated concept search");}, saveConcept: async () => {throw new Error("Unrelated concept save");},
      listReadingUnits: async () => {throw new Error("Unrelated reading-unit list");}, saveReadingUnit: async () => {throw new Error("Unrelated reading-unit save");}}, queries};
  const release = injected ? configureLiteratureServices(owner) : () => undefined;
  if (kind === "queued-bibtex") {
    const gateway = getLiteratureGateway(), importBibtex = gateway.importBibtex.bind(gateway);
    let receipt: ImportJob;
    vi.spyOn(gateway, "importBibtex").mockImplementation(async drafts => {
      receipt = await importBibtex(drafts);
      return { ...receipt, state: "running", results: [] };
    });
    vi.spyOn(gateway, "getImportJob").mockImplementation(async () => receipt);
  }
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const title = `Newly added ${kind} paper`;
  function fill(input: HTMLInputElement | HTMLTextAreaElement, value: string): void {
    const prototype = input instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }
  try {
    await act(async () => root.render(<NotesProvider><LiteratureWorkspace project={undefined} openSource={undefined}
      onSourceOpened={() => undefined} openItemId={undefined} onItemOpened={() => undefined} /></NotesProvider>));
    const oldRow = host.querySelector<HTMLButtonElement>(".literature-row")!;
    await act(async () => oldRow.click());
    const oldId = oldRow.dataset.itemId;
    expect(host.querySelector(".literature-row.selected")?.getAttribute("data-item-id")).toBe(oldId);
    const search = host.querySelector<HTMLInputElement>('[aria-label="搜索文献"]')!;
    await act(async () => { fill(search, oldRow.querySelector(".title")?.textContent ?? "Preview"); });
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 220)); });
    await act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${kind === "manual" ? "手动添加文献" : "从 BibTeX 添加"}"]`)!.click());
    const form = document.querySelector<HTMLFormElement>("#literature-import-popover")!;
    await act(async () => kind === "manual"
      ? fill(form.querySelector<HTMLInputElement>('[placeholder="标题（必填）"]')!, title)
      : fill(form.querySelector<HTMLTextAreaElement>("textarea")!, `@article{new, title={${title}}}\n@article{second, title={Another new paper}}`));
    await act(async () => { form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); });
    if (kind === "queued-bibtex") await act(async () => { await new Promise(resolve => setTimeout(resolve, 350)); });
    expect(search.value).toBe("");
    const selectedRows = host.querySelectorAll(".literature-row.selected");
    expect(selectedRows).toHaveLength(1);
    expect(selectedRows[0]?.textContent).toContain(title);
    expect(selectedRows[0]?.getAttribute("data-item-id")).not.toBe(oldId);
    expect(host.querySelector(".literature-detail")?.textContent).toContain(title);
    // The debounced search/filter refresh must not restore the old selection.
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 220)); });
    expect(host.querySelector(".literature-row.selected")?.textContent).toContain(title);
    if (injected) {
      const id = selectedRows[0]!.getAttribute("data-item-id")!;
      expect(kind === "manual" ? owner.literature.save : owner.literature.importBibtex).toHaveBeenCalledOnce();
      await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="查看文献详情"]')!.click());
      if (kind !== "manual") {
        const confirm = document.querySelector<HTMLButtonElement>('[data-metadata-field="title"] .content-trust-indicator.reviewable')!;
        expect(confirm).not.toBeNull();
        await act(async () => confirm.click());
        expect(owner.literature.confirmMetadataFields).toHaveBeenCalledExactlyOnceWith(id, ["title"]);
        expect(document.querySelector('[data-metadata-field="title"] .content-trust-indicator')).toBeNull();
      }
      await act(async () => document.querySelector('[data-metadata-field="title"]')!
        .dispatchEvent(new MouseEvent("dblclick", {bubbles: true})));
      const input = document.querySelector<HTMLInputElement>('input[aria-label="标题"]')!, edited = `${title} edited`;
      await act(async () => {
        fill(input, edited);
        input.dispatchEvent(new KeyboardEvent("keydown", {bubbles: true, key: "Enter"}));
      });
      expect(owner.literature.updateMetadataField).toHaveBeenCalledExactlyOnceWith(id, {fieldName: "title", value: edited});
      expect(host.querySelector(".literature-row.selected")?.textContent).toContain(edited);
      expect(document.querySelector('[data-metadata-field="title"] .content-trust-indicator')).toBeNull();
    }
  } finally { await act(async () => root.unmount()); host.remove(); release(); }
});
