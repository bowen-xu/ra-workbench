// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createAppRuntime } from "./createAppRuntime";
import { PREVIEW_STORAGE_PREFIX } from "../../core/runtime/appStorage";
import {createIndexedPreviewStorage} from "../../core/runtime/previewIndexedStorage";
import {flushStorage} from "../../core/runtime/storagePersistence";
import { getAppRuntime, runtimeAgentGateway, selectAppRuntime } from "../../core/runtime/appRuntime";
import { getNotesGateway } from "../../core/features/notes/gateways";
import { getWritingGateway } from "@research-assistant/writing-projects-plugin/writingGateway";
import { getLiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";
import { invoke } from "../../core/runtime/platformCore";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { previewLiteratureFixtures } from "./mock/previewLiteratureFixtures";
import { parseSourceLink, resolveSourcePath } from "@research-assistant/core/source";
import { sourceDirectory } from "../../core/features/agent/sourceDirectory";
import { captureEvidenceImage, readEvidenceImage } from "../../core/features/agent/evidenceImageGateway";

class MemoryStorage implements Storage {
  private values = new Map<string, string>();
  get length() { return this.values.size; }
  getItem(key: string) { return this.values.get(key) ?? null; }
  setItem(key: string, value: string) { this.values.set(key, value); }
  removeItem(key: string) { this.values.delete(key); }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
  clear() { this.values.clear(); }
}
describe("full App mock runtime", () => {
  beforeEach(() => {
    vi.stubGlobal("localStorage", new MemoryStorage());
    selectAppRuntime({ mode: "browser", storage: window.localStorage, gateways: {}, dispose() {} });
    vi.stubGlobal("fetch", vi.fn(async (url: string) => {
      if (url !== "/preview/sample.pdf") throw new Error(`unexpected network: ${url}`);
      return new Response("%PDF-1.4\nsynthetic sample", { status: 200 });
    }));
  });
  afterEach(() => { getAppRuntime().dispose(); vi.unstubAllGlobals(); selectAppRuntime({ mode: "browser", storage: window.localStorage, gateways: {}, dispose() {} }); });
  it("selects production ports for ordinary browser mode", async () => {
    const runtime = await createAppRuntime("", window.localStorage);
    expect(runtime.mode).toBe("browser"); expect(runtime.storage).toBe(window.localStorage);
    // Application composition binds the same production domain adapters; Core
    // no longer imports their implementations to discover them itself.
    expect(runtime.gateways.getLiteratureGateway).toBe(getLiteratureGateway());
    expect(runtime.gateways.getWritingGateway).toBe(getWritingGateway());
    expect(runtime.gateways.getAiAssistanceGateway?.startPaperAnalysis).toBeTypeOf("function");
    expect(runtime.gateways.doclingAttachmentMetadata?.describe).toBeTypeOf("function");
  });
  it.each(["?preview=app", "?demo=app"])("uses the isolated mock runtime for %s", async search => {
    window.localStorage.setItem("research-assistant.literature.v1", "business data untouched");
    const runtime = await createAppRuntime(search, window.localStorage);
    expect(runtime.mode).toBe("preview");
    expect(runtime.storage).not.toBe(window.localStorage);
    expect(runtime.gateways.agentWritingFileDeliveryGateway).toBeDefined();
    expect(runtime.gateways.agentPendingWritingApplicationGateway).toBeDefined();
    expect(runtime.gateways.agentPendingWritingUndoGateway).toBeDefined();
    expect(getWritingGateway().supportsCompilation).toBe(true);
    runtime.preview!.configure({delayMs: 0, nextFailure: null});
    await expect(runtime.gateways.getResearchExportGateway!.exportAll("/synthetic/preview"))
      .resolves.toEqual({folderName: "preview-export", fileCount: 1, objectCount: 1, sourceCount: 1});
    await expect(runtime.gateways.getResearchExportGateway!.cancel()).resolves.toBe(true);
    expect(window.localStorage.getItem("research-assistant.literature.v1")).toBe("business data untouched");
  });
  it("isolates every stored key, enumeration and reset from business data", async () => {
    window.localStorage.setItem("research-assistant.literature.v1", "private untouched");
    const preview = await createIndexedPreviewStorage();
    expect(preview.getItem("research-assistant.literature.v1")).toBeNull();
    preview.setItem("research-assistant.literature.v1", "synthetic");
    expect(preview.length).toBe(1); expect(preview.key(0)).toBe("research-assistant.literature.v1");
    await flushStorage(preview);
    expect(window.localStorage.getItem(PREVIEW_STORAGE_PREFIX + "research-assistant.literature.v1")).toBeNull();
    preview.clear(); await flushStorage(preview); expect(window.localStorage.getItem("research-assistant.literature.v1")).toBe("private untouched");
  });
  it.each(["deleted", "archived"] as const)("starts with an unfinished demo seed when the initial manuscript is %s, preserving other content",async status=>{
    window.localStorage.setItem("business-marker","untouched");
    const runtime=await createAppRuntime("?preview=app",window.localStorage);
    runtime.preview!.configure({delayMs:0,nextFailure:null});
    const writing=getWritingGateway(), initialId=runtime.storage.getItem("initial-manuscript")!;
    const retained=await writing.createManuscript({title:"保留的预览稿件"});
    const file=await writing.readFile(retained.id,retained.rootPath);
    await writing.saveFile({manuscriptId:retained.id,relativePath:retained.rootPath,content:"user preview content",expectedSha256:file.sha256});
    await writing.archiveManuscript(initialId);
    if(status==="deleted") await writing.deleteManuscript(initialId);
    runtime.storage.removeItem("agent-seeded");
    const notes=(await getNotesGateway().listPages()).map(page=>page.id);
    runtime.dispose();
    const reopened=await createAppRuntime("?preview=app",window.localStorage);
    const manuscripts=await getWritingGateway().listManuscripts();
    expect(manuscripts.find(manuscript=>manuscript.id===initialId)?.status).toBe(status==="deleted"?undefined:"archived");
    expect((await getWritingGateway().readFile(retained.id,retained.rootPath)).content).toBe("user preview content");
    expect((await getNotesGateway().listPages()).map(page=>page.id)).toEqual(notes);
    expect(window.localStorage.getItem("business-marker")).toBe("untouched");
    expect(reopened.storage.getItem("agent-seeded")).toBeNull();
  });
  it("starts with an empty writing catalog after deleting the initial demo manuscript",async()=>{
    const runtime=await createAppRuntime("?preview=app",window.localStorage);
    runtime.preview!.configure({delayMs:0,nextFailure:null});
    const writing=getWritingGateway(), id=runtime.storage.getItem("initial-manuscript")!;
    await writing.archiveManuscript(id);
    await writing.deleteManuscript(id);
    runtime.storage.removeItem("agent-seeded");
    runtime.dispose();
    const reopened=await createAppRuntime("?preview=app",window.localStorage);
    expect(reopened.mode).toBe("preview");
    expect(await getWritingGateway().listManuscripts()).toEqual([]);
    expect((await getNotesGateway().listPages()).length).toBeGreaterThan(0);
  });
  it("captures and restores annotation images through all preview platform ports", async () => {
    const runtime = await createAppRuntime("?preview=app&demo=source-navigation", window.localStorage);
    runtime.preview!.configure({delayMs: 0, nextFailure: null});
    const bytes = [137, 80, 78, 71, 13, 10, 26, 10, ...new TextEncoder().encode("preview")];
    const image = await captureEvidenceImage(bytes, "image/png", "选区截图");
    expect([...await readEvidenceImage(image)]).toEqual(bytes);
    expect(runtime.storage.getItem("agent-evidence-images")).not.toContain('"bytes"');
    await expect(invoke("agent_read_evidence_image", {request: {...image, assetId: "f".repeat(64)}})).rejects.toThrow("不可用");
    await expect(captureEvidenceImage(bytes, "image/jpeg", "错误格式")).rejects.toThrow("损坏");
    await invoke("agent_release_evidence_image", {captureId: image.captureId});
    await invoke("agent_reconcile_evidence_drafts", {captureIds: [image.captureId]});
    runtime.dispose();
    const restored = await createAppRuntime("?preview=app&demo=source-navigation", window.localStorage);
    restored.preview!.configure({delayMs: 0, nextFailure: null});
    expect([...await readEvidenceImage(image)]).toEqual(bytes);
    await invoke("agent_reconcile_evidence_drafts", {captureIds: []});
    expect([...await readEvidenceImage(image)]).toEqual(bytes);
    expect(window.localStorage.getItem("agent-evidence-images")).toBeNull();
  });
  it("opens a completed source-reference fixture with resolvable PDFs, notes and files in its own preview storage", async () => {
    window.localStorage.setItem("research-assistant.literature.v1", "business data untouched");
    const ordinary = await createIndexedPreviewStorage();
    ordinary.setItem("research-assistant:independent-selected", "keep-ordinary-preview");
    const runtime = await createAppRuntime("?preview=app&demo=source-navigation", window.localStorage);
    const id = runtime.storage.getItem("research-assistant:independent-selected")!;
    const session = runtime.gateways.agentContextGateway!.sessions;
    const history = await session.readHistory({conversationId: id, afterSeq: 0, limit: 100});
    const response = history.events.filter(event => event.payload.type === "textDelta").map(event => event.payload.type === "textDelta" ? event.payload.text : "").join("\n");
    const links = [...response.matchAll(/\]\((source:\/\/[^)]+)\)/gu)].map(match => parseSourceLink(match[1]!));
    expect(links).toHaveLength(6);
    expect(links.some(link => link.locator?.kind === "line" && link.locator.pageNumber === 1)).toBe(true);
    expect(links.some(link => link.locator?.kind === "block" && link.locator.range?.startLine === 2)).toBe(true);
    const directory = sourceDirectory({literature: getLiteratureGateway(), notes: getNotesGateway(), writing: getWritingGateway()});
    for (const link of links) await expect(resolveSourcePath(directory, link)).resolves.toBeDefined();
    const before = (await getNotesGateway().listPages()).length;
    runtime.dispose();
    await createAppRuntime("?preview=app&demo=source-navigation", window.localStorage);
    expect((await getNotesGateway().listPages()).length).toBe(before);
    expect(ordinary.getItem("research-assistant:independent-selected")).toBe("keep-ordinary-preview");
    expect(window.localStorage.getItem("research-assistant.literature.v1")).toBe("business data untouched");
  });
  it("reads each supplied paper's own static PDF and preserves changes when upgrading existing preview data", async () => {
    const runtime = await createAppRuntime("?preview=app", window.localStorage);
    runtime.preview!.configure({ delayMs: 0, nextFailure: null });
    const library = getLiteratureGateway();
    vi.mocked(fetch).mockImplementation(async input => {
      const url = String(input);
      if (!previewLiteratureFixtures.some(fixture => fixture.pdfUrl === url)) throw new Error(`unexpected network: ${url}`);
      const path = resolve(dirname(fileURLToPath(import.meta.url)), "../../../public", url.slice(1));
      return new Response(new Uint8Array(await readFile(path)).buffer);
    });
    for (const fixture of previewLiteratureFixtures) {
      const item = await library.get(fixture.item.id), attachment = item.attachments[0]!;
      const pdf = await library.readAttachment(attachment.id);
      expect(pdf.length).toBe(attachment.byteSize);
      expect(createHash("sha256").update(pdf).digest("hex")).toBe(attachment.contentSha256);
      expect(new TextDecoder().decode(pdf.slice(0, 5))).toBe("%PDF-");
    }
    const first = previewLiteratureFixtures[0]!.item, second = previewLiteratureFixtures[1]!.item;
    await library.updateMetadataField(first.id, { fieldName: "title", value: "custom preview title" });
    await library.trash(second.id);
    // Simulate a prior seed without the new fixture-set marker.
    runtime.storage.setItem("research-assistant.literature.v1", JSON.stringify((await library.list())
      .filter(item => item.id !== previewLiteratureFixtures[2]!.item.id)));
    runtime.storage.removeItem("literature-fixtures-v1");
    await flushStorage(runtime.storage);
    runtime.dispose();
    const reopened = await createAppRuntime("?demo=app", window.localStorage);
    expect((await getLiteratureGateway().get(first.id)).title).toBe("custom preview title");
    expect((await getLiteratureGateway().list()).filter(item => item.id === first.id)).toHaveLength(1);
    expect((await getLiteratureGateway().listTrash()).some(item => item.id === second.id)).toBe(true);
    expect((await getLiteratureGateway().list()).some(item => item.id === second.id)).toBe(false);
    expect((await getLiteratureGateway().get(previewLiteratureFixtures[2]!.item.id)).title).toBe(previewLiteratureFixtures[2]!.item.title);
    await getLiteratureGateway().trash(first.id);
    reopened.dispose();
    await createAppRuntime("?preview=app", window.localStorage);
    expect((await getLiteratureGateway().list()).some(item => item.id === first.id)).toBe(false);
  });
  it("shares edits between lists, details and reconstructed runtimes without reseeding", async () => {
    const runtime = await createAppRuntime("?preview=app", window.localStorage);
    runtime.preview!.configure({ delayMs: 0, nextFailure: null });
    const literature = getLiteratureGateway(), initial = (await literature.list())[0]!;
    await literature.save({ ...initial, title: "updated synthetic title" });
    const notes = getNotesGateway(); const page = await notes.createPage({ title: "persisted note" });
    const writing = getWritingGateway(), manuscript = (await writing.listManuscripts())[0]!;
    const file = await writing.readFile(manuscript.id, "main.tex");
    await writing.saveFile({ manuscriptId: manuscript.id, relativePath: file.relativePath, expectedSha256: file.sha256, content: "% persisted mock disk\n" });
    runtime.dispose();
    const reopened = await createAppRuntime("?preview=app", window.localStorage);
    expect((await getLiteratureGateway().get(initial.id)).title).toBe("updated synthetic title");
    expect((await getNotesGateway().loadDocument(page.page.id)).page.title).toBe("persisted note");
    expect((await getWritingGateway().readFile(manuscript.id, "main.tex")).content).toBe("% persisted mock disk\n");
    expect((await getWritingGateway().listManuscripts()).length).toBe(1);
    expect(reopened.mode).toBe("preview");
  });
  it("fails closed for unsupported platform and Agent methods", async () => {
    const runtime = await createAppRuntime("?preview=app", window.localStorage);
    runtime.preview!.configure({ delayMs: 0, nextFailure: null });
    await expect(invoke("not-a-mock-command")).rejects.toThrow("预览未覆盖平台调用");
    const native = vi.fn();
    const gateway = runtimeAgentGateway("agentChildSpawnGateway", { call: native });
    await expect(gateway.call()).rejects.toThrow("预览未覆盖 Agent 调用"); expect(native).not.toHaveBeenCalled();
  });
  it("injects a one-shot save fault and permits the ordinary retry", async () => {
    const runtime = await createAppRuntime("?preview=app", window.localStorage);
    runtime.preview!.configure({ delayMs: 0, nextFailure: "save" });
    await expect(getNotesGateway().createPage({ title: "retry" })).rejects.toThrow();
    expect((await getNotesGateway().createPage({ title: "retry" })).page.title).toBe("retry");
    expect(runtime.preview!.nextFailure).toBeNull();
  });
  it("freezes current note and paper contexts through the production protocol decoders", async () => {
    const runtime = await createAppRuntime("?preview=app", window.localStorage);
    runtime.preview!.configure({ delayMs: 0, nextFailure: null });
    const page = (await getNotesGateway().listPages())[0]!;
    const note = await runtime.gateways.agentNoteContextGateway!.preview({ pageId: page.id, expectedRevision: page.revision });
    expect(note.source.text).toContain("研究计划");
    const paper = (await getLiteratureGateway().list()).find(item => item.attachments.length)!;
    const prepared = await runtime.gateways.agentPaperContextGateway!.prepare({ paperId: paper.id, attachmentIds: paper.attachments.map(value => value.id) });
    const context = await runtime.gateways.agentPaperContextGateway!.preview({ paperId: paper.id, snapshotId: prepared.snapshotId, expectedContextHash: prepared.contextHash });
    expect(context.items[0]?.text).toContain(paper.title);
    expect(context.attachments[0]?.contentSha256).toBe(paper.attachments[0]?.contentSha256);
  }, 30_000); // Full-runtime context preparation also runs under suite-wide CPU contention.
  it("stops delayed writes when the runtime is disposed", async () => {
    const runtime = await createAppRuntime("?preview=app", window.localStorage);
    runtime.preview!.configure({ delayMs: 20, nextFailure: null });
    const pending = getNotesGateway().createPage({ title: "cancelled write" });
    const expectation = expect(pending).rejects.toThrow("已关闭");
    runtime.dispose(); await expectation;
    const reopened = await createAppRuntime("?preview=app", window.localStorage);
    expect((await getNotesGateway().listPages()).some(page => page.title === "cancelled write")).toBe(false);
    reopened.dispose();
  });
  it("restores the queue paused, retaining its inputs without dispatching them", async () => {
    const runtime = await createAppRuntime("?preview=app", window.localStorage);
    runtime.preview!.configure({ delayMs: 0, nextFailure: null });
    const manuscriptId = runtime.storage.getItem("initial-manuscript")!;
    const catalog = await runtime.gateways.agentCatalogGateway!.list({ binding: { kind: "manuscript", resourceId: manuscriptId }, archived: false, before: null, limit: 20 });
    const scope = { manuscriptId, conversationId: catalog.conversations[0]!.id };
    const queue = runtime.gateways.agentInputQueueGateway!;
    const current = await queue.read(scope);
    await queue.edit({ ...scope, requestId: crypto.randomUUID(), expectedRevision: current.revision, edit: { type: "add", inputId: crypto.randomUUID(), text: "persistent pending input" } });
    runtime.dispose();
    const reopened = await createAppRuntime("?preview=app", window.localStorage);
    const restored = await reopened.gateways.agentInputQueueGateway!.read(scope);
    expect(restored.paused).toBe(true); expect(restored.preparingInputId).toBeNull();
    expect(restored.inputs.map(input => input.text)).toEqual(["persistent pending input"]);
  });
it("runs the note-edit fixture in its own namespace without replacing ordinary preview or business data", async () => {
  const ordinary = await createAppRuntime("?preview=app", window.localStorage);
  ordinary.storage.setItem("ordinary-marker", "keep"); ordinary.dispose();
  window.localStorage.setItem("research-assistant.notes.v1", "business notes untouched");
  const runtime = await createAppRuntime("?preview=app&demo=text-patch", window.localStorage);
  expect(runtime.mode).toBe("preview");
  expect(runtime.gateways.noteTextSessionGateway).toBeDefined();
  const notes = runtime.gateways.getNotesGateway;
  expect(notes).toBeDefined();
  const fixture = await notes!.loadDocument("00000000-0000-4000-8000-000000000022" as import("@research-assistant/core/notes").NotePageId);
  expect(fixture.page.title).toBe("研究笔记"); expect(fixture.page.revision).toBe(1);
  expect(fixture.document.blocks.map(block => block.type)).toEqual(expect.arrayContaining(["heading", "paragraph", "table", "formulaBlock", "quote", "checkListItem", "codeBlock"]));
  expect(ordinary.storage.getItem("ordinary-marker")).toBe("keep");
  expect(runtime.storage.getItem("ordinary-marker")).toBeNull();
  expect(window.localStorage.getItem("research-assistant.notes.v1")).toBe("business notes untouched");
  runtime.dispose();
});

});

it("retains identity and a single event after retaining the selected writing gateway",async()=>{
  const previous=getAppRuntime();
  const source={saveFile:vi.fn(async()=>({content:"saved"}))} as unknown as ReturnType<typeof getWritingGateway>;
  const runtime={mode:"browser" as const,storage:window.localStorage,gateways:{getWritingGateway:source},dispose(){}};
  selectAppRuntime(runtime);
  try {
    const selected=getWritingGateway();
    runtime.gateways.getWritingGateway=selected;
    expect(getWritingGateway()).toBe(selected);
    expect(getWritingGateway()).toBe(selected);
    const dispatch=vi.spyOn(window,"dispatchEvent");
    await Reflect.apply(getWritingGateway().saveFile,selected,[{}]);
    expect(source.saveFile).toHaveBeenCalledOnce();
    expect(dispatch.mock.calls.filter(([event])=>event.type==="research-assistant:writing-notes-changed")).toHaveLength(1);
  } finally {selectAppRuntime(previous);vi.restoreAllMocks();}
});
