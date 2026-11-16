// @vitest-environment happy-dom
import {act, type ComponentProps} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {noteBlockId, notePageId, type ReadingNoteBlock} from "@research-assistant/core/notes";
import type {StructuredReadingUnit} from "@research-assistant/core/knowledge";
import type {StructuredResource, StructuredStartupImportResult} from "@research-assistant/core/structured-resources";
import {getStructuredResourceGateway, type StructuredResourceGateway} from "@research-assistant/workbench-sdk/structured-resources";
import type {AppHbsDocumentGateway} from "@research-assistant/workbench-sdk/hbs";
import {notify} from "@research-assistant/workbench-sdk/feedback";
import {createAppRuntime, createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {ReadingNoteResearchActions} from "../../../../../../../plugins/ra-literature-library/src/features/knowledge/ReadingNoteResearchActions";
import {PaperOverviewPanel} from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperOverviewPanel";
import {panelItem} from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";

const hbs = vi.hoisted(() => ({gateway: undefined as AppHbsDocumentGateway | undefined}));
const native = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("@research-assistant/workbench-sdk/hbs", async importOriginal => {
  const original = await importOriginal<typeof import("@research-assistant/workbench-sdk/hbs")>();
  return {...original, getHbsDocumentGateway: () => hbs.gateway ?? original.getHbsDocumentGateway()};
});
const note: ReadingNoteBlock = {id: noteBlockId("source-note"), pageId: notePageId("core-note"), noteType: "annotation",
  body: "Original note body", color: "#000000", status: "active", isFavorite: false, revision: 1, projectIds: [], tagIds: [],
  createdAt: "created", updatedAt: "updated", source: {paperId: "paper", paperTitle: "Original paper", attachmentId: "attachment",
    contentSha256: "a".repeat(64), anchorId: "anchor", pageIndex: 0, rects: [], quoteExact: "Original quote", quotePrefix: "", quoteSuffix: "", state: "exact"}};
const unit: StructuredReadingUnit = {id: "manual-unit", paperId: "paper", unitKind: "research_question", title: "Original title", body: "Original manual question",
  provenanceKind: "human_original", epistemicRole: "working_note", revision: 3, evidenceCount: 1, createdAt: "created", updatedAt: "updated"};
const capabilities = ["literature.reading-units", "literature.reference-records"].map(id => ({id, version: 1}));
let host: HTMLDivElement, root: Root, releases: (() => void)[];
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear();
  native.invoke.mockReset();
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
  vi.spyOn(notify, "error").mockReturnValue("error"); vi.spyOn(notify, "info").mockReturnValue("info");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host); releases = [];
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); for (const release of releases.reverse()) release();
  hbs.gateway = undefined; vi.restoreAllMocks(); vi.unstubAllGlobals();
});
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, fail) => {resolve = accept; reject = fail;});
  return {promise, resolve, reject};
}
function bind(invoke: (command: string, args: Record<string, unknown>) => Promise<unknown>) {
  releases.push(configureLiteratureServices(createRuntimeLiteraturePort("f".repeat(64), capabilities, () => undefined, invoke)));
}
function button(text: string): HTMLButtonElement {
  const found = [...host.querySelectorAll("button")].find(b => b.textContent === text);
  if (!found) throw new Error(`Missing production control: ${text}`);
  return found;
}
async function input(selector: string, value: string) {
  const element = host.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector);
  if (!element) throw new Error(`Missing production input: ${selector}`);
  await act(async () => {
    const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event("input", {bubbles: true}));
  });
}
function overviewProps(id = "paper"): ComponentProps<typeof PaperOverviewPanel> {
  type Props = ComponentProps<typeof PaperOverviewPanel>;
  return {item: {...panelItem(), id, attachments: []}, onOpenAttachment: vi.fn(), onPaperReviewChanged: vi.fn(async () => undefined),
    aiGateway: {listCandidates: vi.fn(async () => []), getLatestPaperAnalysisWorkflow: vi.fn(async () => undefined),
      getLatestPaperPortraitRun: vi.fn(async () => undefined)} as unknown as NonNullable<Props["aiGateway"]>,
    doclingGateway: {listParseRuns: vi.fn(async () => [])} as unknown as NonNullable<Props["doclingGateway"]>};
}

it("submits the original reading-note form through the selected runtime and waits before closing", async () => {
  const saved = deferred<StructuredReadingUnit>(), invoke = vi.fn(async (command: string) => {
    if (command !== "save_runtime_structured_reading_unit") throw new Error(`Unexpected ${command}`);
    return saved.promise;
  });
  bind(invoke);
  await act(async () => root.render(<ReadingNoteResearchActions note={note}/>));
  await act(async () => button("创建阅读单元").click());
  await input("input", " Original title "); await input("textarea", " Original manual body\n");
  await act(async () => button("创建并关联来源").click());
  expect(invoke).toHaveBeenCalledExactlyOnceWith("save_runtime_structured_reading_unit", {packageKey: "f".repeat(64), draft: {
    paperId: "paper", anchorId: "anchor", evidenceStance: "context", evidenceRationale: "由用户从阅读笔记创建", unitKind: "other",
    title: " Original title ", body: " Original manual body\n", epistemicRole: "working_note", changeReason: "reading_note_capture",
  }});
  expect(button("创建并关联来源").disabled).toBe(true); expect(host.querySelector("textarea")).not.toBeNull();
  expect(notify.info).not.toHaveBeenCalled();
  await act(async () => saved.resolve(unit));
  expect(host.querySelector("textarea")).toBeNull(); expect(notify.info).toHaveBeenCalledWith("人工阅读单元已创建并关联原文来源");
});

it("retains the original reading-note draft and safe native failure without replay", async () => {
  const invoke = vi.fn(async () => {throw {code: "storage_error", message: "Original reading save failed", instanceId: "reading-host", requestId: "reading-request",
    readingUnits: {phase: "commit", outcome: "unknown", readingUnitId: "manual-unit", problem: {kind: "database"}}};});
  bind(invoke); await act(async () => root.render(<ReadingNoteResearchActions note={note}/>));
  await act(async () => button("创建阅读单元").click());
  await input("input", "Retained title"); await input("textarea", "Retained body");
  await act(async () => button("创建并关联来源").click());
  expect(host.querySelector<HTMLInputElement>("input")?.value).toBe("Retained title");
  expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("Retained body");
  expect(button("创建并关联来源").disabled).toBe(false); expect(invoke).toHaveBeenCalledOnce();
  expect(notify.error).toHaveBeenCalledWith(expect.stringMatching(/storage_error.*reading-request.*readingUnits.outcome=unknown.*草稿仍保留/u));
});

it("loads manual units for the original paper only and renders the existing overview fields", async () => {
  const invoke = vi.fn(async (command: string) => {
    if (command === "list_runtime_structured_reading_units") return [unit];
    if (command === "list_runtime_reference_records") return [];
    throw new Error(`Unexpected ${command}`);
  });
  bind(invoke); await act(async () => root.render(<PaperOverviewPanel {...overviewProps()}/>));
  expect(invoke.mock.calls.filter(([command]) => command === "list_runtime_structured_reading_units")).toHaveLength(1);
  expect(invoke).toHaveBeenCalledWith("list_runtime_structured_reading_units", {packageKey: "f".repeat(64), projectId: undefined, paperId: "paper"});
  expect(host.querySelector('[data-reading-field="research_question"]')?.textContent).toContain(unit.body);
});

it("does not install a late old-paper manual unit into the newly selected production overview", async () => {
  const old = deferred<readonly StructuredReadingUnit[]>();
  const invoke = vi.fn(async (command: string, args: Record<string, unknown>) => {
    if (command === "list_runtime_structured_reading_units") return args.paperId === "paper" ? old.promise : [{...unit, id: "next-unit", paperId: "next-paper", body: "Next paper body"}];
    if (command === "list_runtime_reference_records") return [];
    throw new Error(`Unexpected ${command}`);
  });
  bind(invoke); await act(async () => root.render(<PaperOverviewPanel {...overviewProps()}/>));
  await act(async () => root.render(<PaperOverviewPanel {...overviewProps("next-paper")}/>));
  expect(host.textContent).toContain("Next paper body");
  await act(async () => old.resolve([unit]));
  expect(host.textContent).toContain("Next paper body"); expect(host.textContent).not.toContain(unit.body);
});

it("uses the original managed-overview retry after a deferred startup and reaches real list, sources and review components", async () => {
  const content = {kind: "paper_overview_unit_v1", schemaVersion: 1, profileId: "reviewed_profile_composition_v1", fields: [
    {field: "research_question", status: "filled", statements: [{id: "manual-statement", text: "Managed original body", epistemicRole: "working_note", evidenceItemIds: []}]},
  ]};
  const reading: StructuredResource = {id: "managed-reading", paperId: "paper", kind: "structuredReading", title: "Reading", sourceUri: "source://library/paper/reading",
    revision: 1, content, reviewBaseline: content, pendingFieldIds: [], reviewedFieldIds: []};
  let ready = false;
  const importStartup = vi.fn(async (): Promise<StructuredStartupImportResult> => {
    if (!ready) return {paperIds: [], importedPaperCount: 0, importedTemplateCount: 0, deferredPaperIds: ["paper"]};
    return {paperIds: ["paper"], importedPaperCount: 1, importedTemplateCount: 0};
  });
  const list = vi.fn(async () => {if (!ready) throw new Error("Original selected reading owner unavailable"); return [reading];});
  const getSources = vi.fn(async () => []), unexpected = vi.fn(async (): Promise<never> => {throw new Error("Unexpected structured write");});
  const gateway: StructuredResourceGateway = {importStartup, list, getSources, get: async () => reading, getVersion: async () => reading,
    createTemplate: unexpected, write: unexpected, review: unexpected, restoreHistory: unexpected, bindTemplate: unexpected, importLegacy: unexpected};
  hbs.gateway = {read: async () => ({version: 1, saved: content, baseline: content}), readPreservation: async () => null,
    commit: unexpected, savePreservation: async () => ({revision: 1})};
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {getStructuredResourceGateway: gateway}, dispose() {}});
  bind(async command => {
    if (command === "list_runtime_structured_reading_units" || command === "list_runtime_reference_records") return [];
    throw new Error(`Unexpected ${command}`);
  });
  await act(async () => root.render(<PaperOverviewPanel {...overviewProps()}/>));
  expect(importStartup).toHaveBeenCalledOnce(); expect(list).toHaveBeenCalledOnce(); expect(getSources).not.toHaveBeenCalled();
  ready = true;
  await act(async () => button("重试迁移阅读内容").click());
  expect(importStartup).toHaveBeenCalledTimes(2); expect(list).toHaveBeenCalledTimes(2);
  expect(getSources).toHaveBeenCalledWith(reading.id);
  expect(host.textContent).toContain("Managed original body");
  expect(host.textContent).not.toContain("Original selected reading owner unavailable");
  expect(gateway.write).not.toHaveBeenCalled();
});

it.each(["structured_resource_import_startup", "structured_get_resource_sources"])("shows finite Native %s diagnostics through the real Core gateway in the original overview", async failedCommand => {
  const packageKey = "a".repeat(64);
  const reading: StructuredResource = {id: "managed-reading", paperId: "paper", kind: "structuredReading", title: "Reading",
    sourceUri: "source://library/paper/managed-reading", revision: 1, content: null, reviewBaseline: null, pendingFieldIds: [], reviewedFieldIds: []};
  native.invoke.mockImplementation(async (command: string) => {
    if (command === failedCommand) throw {code: "invalid_input", message: "Original frozen evidence failed", pluginId: "literature_library",
      packageKey, instanceId: "source-instance", dataError: {code: "plugin_data_sql_failed", phase: "read",
        resourceId: "literature.reading-evidence.snapshot-rows", resourceVersion: 1, safeCause: "ConstraintViolation",
        sqliteCode: "ConstraintViolation", sqliteExtendedCode: 787, body: "PRIVATE_BODY", sql: "PRIVATE_SQL"},
      path: "/private/source", headers: {authorization: "PRIVATE_HEADER"}};
    if (command === "structured_resource_import_startup") return {paperIds: ["paper"], importedPaperCount: 0, importedTemplateCount: 0};
    if (command === "structured_list_resources") return [reading];
    throw new Error(`Unexpected native command ${command}`);
  });
  // Retain the real SDK/Core command methods; only external I/O and the change
  // subscription use this original App preview's data/lifecycle ports.
  const coreGateway = getStructuredResourceGateway();
  vi.spyOn(globalThis, "fetch").mockImplementation(async input => {
    expect(String(input)).toBe("/preview/sample.pdf");
    return new Response("%PDF-1.4\nSynthetic test asset", {headers: {"Content-Type": "application/pdf"}});
  });
  const preview = await createAppRuntime("?preview=app", localStorage);
  releases.push(() => preview.dispose());
  selectAppRuntime({...preview, invoke: native.invoke, gateways: {...preview.gateways,
    getStructuredResourceGateway: {...coreGateway, subscribeChanges: async () => () => undefined},
  }});
  bind(async command => {
    if (command === "list_runtime_structured_reading_units" || command === "list_runtime_reference_records") return [];
    throw new Error(`Unexpected literature command ${command}`);
  });
  await act(async () => root.render(<PaperOverviewPanel {...overviewProps()}/>));
  const alert = [...host.querySelectorAll('[role="alert"]')].find(node => node.textContent?.includes("Original frozen evidence failed"));
  expect(alert).toBeDefined(); expect(alert?.textContent).toContain("code=invalid_input");
  expect(alert?.textContent).toContain(`packageKey=${packageKey}`); expect(alert?.textContent).toContain("instanceId=source-instance");
  expect(alert?.textContent).toContain("dataError.resourceId=literature.reading-evidence.snapshot-rows");
  expect(alert?.textContent).toContain("dataError.resourceVersion=1"); expect(alert?.textContent).toContain("dataError.sqliteExtendedCode=787");
  expect(alert?.textContent).toContain("原内容已保留，恢复前暂停生成");
  expect(alert?.textContent).not.toMatch(/PRIVATE_|\/private|requestId=/u);
  expect(button("重试迁移阅读内容")).toBeDefined();
  expect(native.invoke.mock.calls.filter(([command]) => command === failedCommand)).toHaveLength(1);
  expect(native.invoke.mock.calls.some(([command]) => /write|review|save/u.test(String(command)))).toBe(false);
});
