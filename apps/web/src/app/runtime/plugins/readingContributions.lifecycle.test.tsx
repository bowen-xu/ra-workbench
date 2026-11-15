// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {PluginReadingContributions} from "@research-assistant/workbench-sdk/activation";
import type {AppHbsDocumentGateway} from "@research-assistant/workbench-sdk/hbs";
import type {StructuredResourceGateway} from "@research-assistant/workbench-sdk/structured-resources";
import {noteBlockId, notePageId, type ReadingNoteBlock} from "@research-assistant/core/notes";
import type {StructuredResource} from "@research-assistant/core/structured-resources";
import {ReadingNotePresentationProvider} from "../../../core/public/notePresentation";
import {ReadingNoteSourceSection} from "../../../core/features/notes/reading-notes/ReadingNoteSourceSection";
import {ReadingNoteResearchActions} from "@research-assistant/literature-library-plugin/research";
import {ReadingTemplateDialog} from "@research-assistant/literature-library-plugin/navigation";
import {createRuntimePluginWorkbench} from "./runtimePluginWorkbench";

const writes = vi.hoisted(() => ({reading: vi.fn(async () => {throw new Error("synthetic reading failure");}),
  concept: vi.fn(async () => {throw new Error("synthetic concept failure");})}));
vi.mock("@research-assistant/workbench-sdk/literature-runtime", () => ({
  resolveKnowledgeRuntimeGateway: (fallback: object) => ({...fallback,
    saveReadingUnit: writes.reading, captureConceptUsage: writes.concept, searchConcepts: async () => []}),
}));
const roots: Root[] = [];
beforeEach(() => {vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear();});
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  document.body.replaceChildren(); vi.clearAllMocks(); vi.unstubAllGlobals();
});
const note: ReadingNoteBlock = {id: noteBlockId("synthetic-reading-note"), pageId: notePageId("synthetic-core-note"),
  noteType: "annotation", body: "Synthetic retained draft", color: "#000000", status: "active", isFavorite: false,
  revision: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now", source: {
    paperId: "paper", paperTitle: "Synthetic paper", attachmentId: "attachment", contentSha256: "a".repeat(64),
    anchorId: "anchor", pageIndex: 0, rects: [], quoteExact: "Synthetic source", quotePrefix: "", quoteSuffix: "", state: "exact",
  }};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(accept => {resolve = accept;});
  return {promise, resolve};
}
async function setup(reading: PluginReadingContributions) {
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  const gate = deferred<boolean>(), prepareClose = vi.fn(() => gate.promise);
  workbench.attachNavigation({open: async () => undefined, prepareClose});
  const manifest = decodePluginManifest({formatVersion: 1, id: "literature_library", version: "1.0.0", apiVersion: 1,
    entry: "assets/index.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}});
  await workbench.activate({plugins: [{manifest, module: {activate(context) {
    context.registerView({id: "library", title: "Literature", reading, render: () => null});
  }}, entryUrl: "/synthetic/index.js", styles: []}], bindings: [], unavailableOptional: []});
  const registered = workbench.views.getSnapshot()[0];
  if (!registered?.reading) throw new Error("Fixture reading contribution absent");
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container); roots.push(root);
  return {workbench, gate, prepareClose, reading: registered.reading, root, container};
}
const button = (text: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(value => value.textContent === text)!;

it.each(["reading", "concept"] as const)("keeps the real %s draft mounted and prevents new domain writes while its owner is stopping or failed", async kind => {
  const f = await setup({ReadingActions: ReadingNoteResearchActions}), ResearchActions = f.reading.ReadingActions!;
  const color = vi.fn();
  await act(async () => f.root.render(<ReadingNotePresentationProvider ResearchActions={ResearchActions}>
    <ReadingNoteSourceSection note={note} onColorChange={color}/>
  </ReadingNotePresentationProvider>));
  await act(async () => button(kind === "reading" ? "创建阅读单元" : "添加概念").click());
  const draft = f.container.querySelector<HTMLTextAreaElement>("textarea")!;
  const initialDraft = draft.value;
  expect(draft).not.toBeNull();
  let stopping!: Promise<void>;
  await act(async () => {stopping = f.workbench.deactivate("literature_library");});
  const rejected = expect(stopping).rejects.toMatchObject({code: "plugin_deactivation_failed"});
  expect(f.workbench.getSnapshot()[0]?.state).toBe("deactivating");
  expect(f.container.querySelector(".reading-note-research-actions")?.hasAttribute("inert")).toBe(true);
  expect((f.container.querySelector(".reading-note-research-actions") as HTMLElement).hidden).toBe(true);
  const submit = button(kind === "reading" ? "创建并关联来源" : "正在保存…");
  expect(submit.disabled).toBe(true);
  await act(async () => submit.click());
  expect(writes.reading).not.toHaveBeenCalled(); expect(writes.concept).not.toHaveBeenCalled();
  await act(async () => {f.gate.resolve(false); await rejected;});
  expect(f.workbench.getSnapshot()[0]?.state).toBe("failed");
  expect(f.container.querySelector("textarea")).toBe(draft); expect(draft.value).toBe(initialDraft);
  expect(f.workbench.views.getSnapshot()).toHaveLength(1);
  expect(f.container.querySelector<HTMLInputElement>('[aria-label="阅读笔记颜色"]')?.disabled).toBe(false);
});

function templateFixture() {
  const content = (question: string) => ({kind: "reading_template_v1", schemaVersion: 1, name: "Synthetic template",
    fields: [{id: "question", title: "Question", question}]});
  let resource: StructuredResource = {id: "synthetic-template", kind: "readingTemplate", title: "Synthetic template",
    sourceUri: "source://agents/synthetic-template", revision: 3, content: content("new"), reviewBaseline: content("old"),
    pendingFieldIds: ["question"], reviewedFieldIds: []};
  const unused = async (): Promise<never> => {throw new Error("Unexpected synthetic fixture operation");};
  const createTemplate = vi.fn(async () => resource), review = vi.fn(async () => resource);
  const write = vi.fn<StructuredResourceGateway["write"]>(async draft => {
    resource = {...resource, content: JSON.parse(draft.content) as unknown, revision: resource.revision + 1};
    return {status: "applied", resource, errors: []};
  });
  const gateway: StructuredResourceGateway = {list: async () => [resource], get: async () => resource,
    getSources: async () => [], getVersion: async () => resource, createTemplate, write, review,
    restoreHistory: unused, bindTemplate: unused, importLegacy: unused, importStartup: unused};
  const snapshot = () => ({version: resource.revision, saved: resource.content, baseline: resource.reviewBaseline});
  const commit = vi.fn<AppHbsDocumentGateway["commit"]>(async (_kind, _id, request) => {
    resource = {...resource, revision: resource.revision + 1, content: request.saved, reviewBaseline: request.baseline};
    return {type: "receipt", requestId: request.id, snapshot: snapshot()};
  });
  const raw: AppHbsDocumentGateway = {read: async () => snapshot(), commit,
    readPreservation: async () => null, savePreservation: async () => ({revision: 1})};
  return {gateway, raw, createTemplate, write, review, commit};
}

it("keeps the original template dialog and closes its new-input and already-portaled review paths when stopping", async () => {
  const fixture = templateFixture(), close = vi.fn();
  const f = await setup({ReadingTemplateDialog: props => <ReadingTemplateDialog {...props} gateway={fixture.gateway} hbsGateway={fixture.raw}/>});
  const Dialog = f.reading.ReadingTemplateDialog!;
  await act(async () => f.root.render(<Dialog onClose={close}/>));
  const draft = document.querySelector<HTMLInputElement>('[aria-label="新模板名称"]')!;
  expect(draft).not.toBeNull();
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(draft, "retained new template");
    draft.dispatchEvent(new Event("input", {bubbles: true}));
  });
  await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="更多阅读审阅操作"]')!.click());
  const portaledKeep = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(value => value.textContent === "Keep 未校验")!;
  expect(portaledKeep).toBeDefined();
  expect(document.querySelector(".structured-template-scroll")?.contains(portaledKeep)).toBe(false);
  let stopping!: Promise<void>;
  await act(async () => {stopping = f.workbench.deactivate("literature_library");});
  const rejected = expect(stopping).rejects.toMatchObject({code: "plugin_deactivation_failed"});
  expect(document.querySelector(".structured-template-scroll")?.hasAttribute("inert")).toBe(true);
  expect(draft.disabled).toBe(true); expect(button("新建全局模板草稿").disabled).toBe(true);
  await act(async () => {button("新建全局模板草稿").click(); portaledKeep.click();});
  expect(fixture.createTemplate).not.toHaveBeenCalled(); expect(fixture.review).not.toHaveBeenCalled(); expect(fixture.commit).not.toHaveBeenCalled();
  await act(async () => {f.gate.resolve(false); await rejected;});
  expect(document.querySelector('[aria-label="新模板名称"]')).toBe(draft); expect(draft.value).toBe("retained new template");
  expect(document.querySelector<HTMLButtonElement>('[aria-label="关闭阅读模板"]')?.disabled).toBe(false);
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="关闭阅读模板"]')!.click());
  expect(close).toHaveBeenCalledOnce();
});

it("lets an already queued local template save finish on its original owner after input is disabled", async () => {
  const fixture = templateFixture();
  const f = await setup({ReadingTemplateDialog: props => <ReadingTemplateDialog {...props} gateway={fixture.gateway} hbsGateway={fixture.raw}/>});
  const Dialog = f.reading.ReadingTemplateDialog!;
  await act(async () => f.root.render(<Dialog onClose={vi.fn()}/>));
  const input = document.querySelector<HTMLInputElement>('[aria-label="模板名称"]')!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "Queued local edit");
    input.dispatchEvent(new Event("input", {bubbles: true}));
  });
  expect(fixture.write).not.toHaveBeenCalled();
  let stopping!: Promise<void>;
  await act(async () => {stopping = f.workbench.deactivate("literature_library");});
  const rejected = expect(stopping).rejects.toMatchObject({code: "plugin_deactivation_failed"});
  await act(async () => {await vi.waitFor(() => expect(fixture.write).toHaveBeenCalledOnce());});
  expect(JSON.parse(fixture.write.mock.calls[0]![0].content)).toMatchObject({name: "Queued local edit"});
  await act(async () => {f.gate.resolve(false); await rejected;});
  expect(document.querySelector('[aria-label="模板名称"]')).toBe(input); expect(input.disabled).toBe(true);
});
