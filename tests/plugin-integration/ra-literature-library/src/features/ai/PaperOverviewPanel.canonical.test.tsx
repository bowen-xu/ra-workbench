// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { getAppStorage, selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import type { StructuredResourceGateway } from "../../../../../../apps/web/src/core/services/structuredResourceGateway";
import type { AppHbsDocumentGateway } from "../../../../../../apps/web/src/core/services/hbsDocumentGateway";
import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import type { ResearchKnowledgeGateway } from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import type { DoclingGateway } from "../../../../../../apps/web/src/core/services/doclingGateway";
import { overviewCandidate, panelItem } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import { PaperOverviewPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperOverviewPanel";

const mode = vi.hoisted(() => ({canonical: true, missing: false, loading: false, error: undefined as string | undefined, reload: vi.fn(),
  reading: undefined as StructuredResource | undefined, gateway: undefined as StructuredResourceGateway | undefined, changed: vi.fn(),hbs:undefined as AppHbsDocumentGateway|undefined}));
const notifications = vi.hoisted(() => ({error: vi.fn(), info: vi.fn(), success: vi.fn()}));
const prepareTemplate=vi.hoisted(()=>vi.fn(async()=>{throw new Error("请先生成文献画像与本文阅读模板。");}));
vi.mock("../../../../../../../plugins/ra-literature-library/src/features/ai/resources/prepareInitialReadingTemplate",()=>({prepareInitialReadingTemplate:prepareTemplate}));
beforeEach(()=>{
  const values=new Map<string,string>();
  selectAppStorage({get length(){return values.size;},key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()});
  mode.hbs={read:async()=>({version:mode.reading?.revision??0,saved:mode.reading?.content??null,baseline:mode.reading?.reviewBaseline??null}),
    commit:async()=>{throw new Error("unexpected HBS commit");},readPreservation:async()=>null,savePreservation:async()=>({revision:1})};
});
afterEach(() => { mode.canonical = true; mode.missing = false; mode.loading = false; mode.error = undefined; mode.reading = undefined; mode.gateway = undefined; vi.clearAllMocks(); });

vi.mock("../../../../../../../plugins/ra-literature-library/src/features/ai/resources/usePaperStructuredResources", () => {
  const resource = {
    id: "reading", kind: "structuredReading", paperId: "paper", sourceUri: "source://library/paper/reading",
    title: "结构化阅读", revision: 0, content: null, reviewBaseline: null,
    pendingFieldIds: [], reviewedFieldIds: [],
  };
  const state = {
    available: true, loading: false, error: undefined, template: undefined,
    portrait: undefined, reading: resource, paperTemplate: undefined, sources: [],
    reload: vi.fn(), changed: vi.fn(), gateway: {getSources: async () => [],get:async()=>mode.reading??resource},
  };
  return {usePaperStructuredResources: () => ({...state, available: mode.canonical, loading: mode.loading, error: mode.error,
    reload: mode.reload, changed: mode.changed, gateway: mode.gateway?{...mode.gateway,get:async()=>mode.reading??resource}:state.gateway,
    reading: mode.canonical && !mode.missing ? mode.reading ?? resource : undefined})};
});
vi.mock("../../../../../../apps/web/src/core/services/hbsDocumentGateway",()=>({getHbsDocumentGateway:()=>mode.hbs,getPendingHbsDocumentCommit:()=>undefined}));
vi.mock("../../../../../../../plugins/ra-literature-library/src/features/ai/PaperPortraitPanel", async () => {
  const {useEffect} = await import("react");
  return {PaperPortraitPanel: ({onConfirmedTemplateChanged}: {onConfirmedTemplateChanged: (template: undefined, state: string) => void}) => {
    useEffect(() => onConfirmedTemplateChanged(undefined, "absent"), [onConfirmedTemplateChanged]);
    return null;
  }};
});
vi.mock("../../../../../../../plugins/ra-literature-library/src/features/ai/ReadingProfileSelector", async () => {
  const {useEffect} = await import("react");
  return {ReadingProfileSelector: ({onSelectionChanged}: {
    readonly onSelectionChanged: (fields: readonly string[], selection: unknown) => void;
  }) => {
    useEffect(() => onSelectionChanged(["research_question"], {paperId: "paper", profiles: [
      {profileId: "legacy", profileVersion: 1},
    ], revision: 9, isPersisted: true}), [onSelectionChanged]);
    return <section aria-label="旧阅读框架配置"/>;
  }};
});
vi.mock("../../../../../../../plugins/ra-literature-library/src/features/ai/resources/ReadingTemplateDialog", () => ({
  ReadingTemplateDialog: () => <section role="dialog" aria-label="本文阅读模板"/>,
}));
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({notify: notifications}));

it("shows canonical content without reading or rewriting obsolete revision visibility preferences", async () => {
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const storage = getAppStorage(), key = "research-assistant.reading-presentation.v1:paper";
  const saved = JSON.stringify({fields: [], hiddenUnitIds: [], hiddenResourceVersions: [{resourceId: "reading", revision: 7}]});
  storage.setItem(key, saved);
  const read = vi.spyOn(storage, "getItem"), write = vi.spyOn(storage, "setItem");
  const payload = overviewCandidate("saved", "paper").revisions[0]!.payload;
  mode.reading = {id: "reading", kind: "structuredReading", paperId: "paper", sourceUri: "source://library/paper/reading",
    title: "结构化阅读", revision: 7, content: payload, reviewBaseline: payload,
    pendingFieldIds: [], reviewedFieldIds: []};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<PaperOverviewPanel item={panelItem()}
      aiGateway={{listCandidates: async () => [], getLatestPaperAnalysisWorkflow: async () => undefined} as unknown as AiAssistanceGateway}
      knowledgeGateway={{listReadingUnits: async () => []} as unknown as ResearchKnowledgeGateway}
      doclingGateway={{} as DoclingGateway} onOpenAttachment={vi.fn()} onPaperReviewChanged={async () => undefined}/>));
    expect(host.querySelectorAll(".reading-category-card")).toHaveLength(1);
    expect(host.querySelector('[aria-label="清空结构化阅读记录"]')).not.toBeNull();
    expect(read).not.toHaveBeenCalledWith(key);
    expect(write.mock.calls.some(([writtenKey]) => writtenKey === key)).toBe(false);
    expect(storage.getItem(key)).toBe(saved);
  } finally {
    await act(async () => root.unmount()); host.remove(); read.mockRestore(); write.mockRestore();
  }
});

it.each([false, true])("clears the saved reading through CAS and preserves visible data on failure (failure=%s)", async failure => {
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const payload = overviewCandidate("saved", "paper").revisions[0]!.payload;
  mode.reading = {id: "reading", kind: "structuredReading", paperId: "paper", sourceUri: "source://library/paper/reading",
    title: "结构化阅读", revision: 7, content: payload, reviewBaseline: payload,
    pendingFieldIds: [], reviewedFieldIds: []};
  const cleared = {...mode.reading, revision: 8, content: null, reviewBaseline: null};
  const review = vi.fn(async () => {if (failure) throw new Error("version conflict"); return cleared;});
  mode.gateway = {review, getSources: async () => []} as unknown as StructuredResourceGateway;
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<PaperOverviewPanel item={panelItem()}
      aiGateway={{listCandidates: async () => [], getLatestPaperAnalysisWorkflow: async () => undefined} as unknown as AiAssistanceGateway}
      knowledgeGateway={{listReadingUnits: async () => []} as unknown as ResearchKnowledgeGateway}
      doclingGateway={{} as DoclingGateway} onOpenAttachment={vi.fn()} onPaperReviewChanged={async () => undefined}/>));
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="清空结构化阅读记录"]')!.click());
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "确认清空")!.click());
    expect(review).toHaveBeenCalledWith(expect.objectContaining({resourceId: "reading", expectedRevision: 7, action: "clear"}));
    if (failure) {
      expect(mode.changed).not.toHaveBeenCalled();
      expect(host.querySelectorAll(".reading-category-card")).toHaveLength(1);
      expect(notifications.error).toHaveBeenCalledWith(expect.stringContaining("version conflict"));
    } else expect(mode.changed).toHaveBeenCalledWith(cleared);
  } finally {await act(async () => root.unmount()); host.remove();}
});

it("removes the separate template entry and hides the legacy profile configuration", async () => {
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const aiGateway = {
    listCandidates: async () => [], getLatestPaperAnalysisWorkflow: async () => undefined,
  } as unknown as AiAssistanceGateway;
  const knowledgeGateway = {listReadingUnits: async () => []} as unknown as ResearchKnowledgeGateway;
  try {
    await act(async () => root.render(<PaperOverviewPanel
      item={panelItem()} aiGateway={aiGateway} knowledgeGateway={knowledgeGateway}
      doclingGateway={{} as DoclingGateway} onOpenAttachment={vi.fn()}
      onPaperReviewChanged={async () => undefined}
    />));
    expect(host.querySelector('[aria-label="旧阅读框架配置"]')).toBeNull();
    expect(host.querySelector('button[aria-label="本文阅读模板"]')).toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});


it("prepares the first resource template directly and never falls back to a stale legacy profile", async () => {
  mode.canonical = false;
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const startPaperAnalysis = vi.fn();
  const aiGateway = {listCandidates: async () => [],
    getLatestPaperAnalysisWorkflow: async () => undefined, startPaperAnalysis} as unknown as AiAssistanceGateway;
  const knowledgeGateway = {listReadingUnits: async () => []} as unknown as ResearchKnowledgeGateway;
  const render = (): Promise<void> => act(async () => root.render(<PaperOverviewPanel
    item={panelItem()} aiGateway={aiGateway} knowledgeGateway={knowledgeGateway}
    doclingGateway={{} as DoclingGateway} onOpenAttachment={vi.fn()}
    onPaperReviewChanged={async () => undefined}
  />));
  try {
    await render();
    expect(host.querySelector('[aria-label="旧阅读框架配置"]')).not.toBeNull();
    const preferenceKey = "research-assistant.reading-presentation.v1:paper";
    const oldPreference = JSON.stringify({fields: [], hiddenUnitIds: [], hiddenResourceVersions: [{resourceId: "reading", revision: 0}]});
    getAppStorage().setItem(preferenceKey, oldPreference);
    mode.canonical = true;
    await render();
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="提取结构化阅读"]')!.click());
    expect(startPaperAnalysis).not.toHaveBeenCalled();
    expect(host.querySelector('[role="dialog"][aria-label="本文阅读模板"]')).toBeNull();
    expect(notifications.error).toHaveBeenCalledWith(expect.stringContaining("请先生成文献画像"));
    expect(prepareTemplate).toHaveBeenCalledWith(expect.any(Object),"paper");
    expect(getAppStorage().getItem(preferenceKey)).toBe(oldPreference);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});


it.each(["loading", "failed"])("keeps legacy reading data read-only while canonical restoration is %s", async phase => {
  mode.missing = true;
  mode.loading = phase === "loading";
  mode.error = phase === "failed" ? "旧显示配置无法解析" : undefined;
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const reviseCandidate = vi.fn(), confirmOverviewFields = vi.fn(), startPaperAnalysis = vi.fn();
  const aiGateway = {listCandidates: async () => [overviewCandidate("legacy", "paper")],
    getLatestPaperAnalysisWorkflow: async () => undefined,
    reviseCandidate, confirmOverviewFields, startPaperAnalysis} as unknown as AiAssistanceGateway;
  const knowledgeGateway = {listReadingUnits: async () => []} as unknown as ResearchKnowledgeGateway;
  try {
    await act(async () => root.render(<PaperOverviewPanel
      item={panelItem()} aiGateway={aiGateway} knowledgeGateway={knowledgeGateway}
      doclingGateway={{} as DoclingGateway} onOpenAttachment={vi.fn()}
      onPaperReviewChanged={async () => undefined}
    />));
    expect(host.querySelector('[data-review-candidate]')).toBeNull();
    expect(host.querySelector('.structured-reading-fields')).toBeNull();
    expect(host.querySelector('[aria-label="旧阅读框架配置"]')).toBeNull();
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="提取结构化阅读"]')!.disabled).toBe(true);
    if (phase === "failed") {
      expect(host.querySelector('[role="alert"]')?.textContent).toContain(mode.error);
      const retry = [...host.querySelectorAll("button")].find(button => button.textContent === "重试迁移阅读内容")!;
      await act(async () => retry.click());
      expect(mode.reload).toHaveBeenCalledOnce();
    } else expect(host.querySelector('[role="status"]')?.textContent).toContain("加载中");
    expect(host.querySelector('[aria-label="阅读版本管理"]')).toBeNull();
    expect(host.querySelector('[aria-label="历史阅读版本"]')).toBeNull();
    expect(reviseCandidate).not.toHaveBeenCalled();
    expect(confirmOverviewFields).not.toHaveBeenCalled();
    expect(startPaperAnalysis).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
