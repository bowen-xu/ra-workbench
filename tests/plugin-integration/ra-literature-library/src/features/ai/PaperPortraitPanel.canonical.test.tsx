// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { DEFAULT_PORTRAIT_TEMPLATE, PORTRAIT_DIMENSIONS, type AiCandidate } from "@research-assistant/core/ai";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import type { StructuredResourceGateway } from "../../../../../../apps/web/src/core/services/structuredResourceGateway";
import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { overviewCandidate, panelItem } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import { PaperPortraitPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperPortraitPanel";

vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({notify: {error: vi.fn(), info: vi.fn()}}));

it.each([true, false])("blocks both portrait generation entrances during canonical recovery (missing resource=%s)", async missing => {
  (globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const values = new Map<string, string>();
  selectAppStorage({get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear()});
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const original = overviewCandidate("portrait-history", "paper");
  const portrait = {kind: "paper_portrait_v1" as const, schemaVersion: 1 as const, coverage: "来源摘录",
    claims: PORTRAIT_DIMENSIONS.map((dimension, index) => ({id: `claim-${index}`, dimension,
      label: "不确定", description: "材料不足", certainty: "uncertain" as const, evidence: []})),
    template: DEFAULT_PORTRAIT_TEMPLATE};
  const historical: AiCandidate = {...original, candidateKind: "paper_portrait_v1",
    revisions: [{...original.revisions[0]!, payload: portrait}]};
  const resource: StructuredResource = {id: "portrait", kind: "portrait", paperId: "paper",
    sourceUri: "source://library/paper/portrait", title: "文献画像", revision: 1, content: portrait,
    reviewBaseline: portrait, pendingFieldIds: [], reviewedFieldIds: []};
  const previewTask = vi.fn(), startRun = vi.fn();
  const gateway = {listCandidates: async () => [historical], previewTask, startRun} as unknown as AiAssistanceGateway;
  try {
    await act(async () => root.render(<PaperPortraitPanel item={panelItem()} gateway={gateway}
      resourceManaged resourceBlocked={!missing} {...(missing ? {} : {managedResource: resource,
        resourceGateway: {getSources: async () => []} as unknown as StructuredResourceGateway,
        onResourceChanged: vi.fn()})}
    />));
    if (missing) {
      expect(host.querySelector("dl")).toBeNull();
      expect(host.querySelector<HTMLButtonElement>('button[aria-label="生成画像"]')!.disabled).toBe(true);
    }
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="查看画像详情"]')!.click());
    const dialog = document.querySelector<HTMLElement>('[role="dialog"][aria-label="文献画像详情"]')!;
    const generate = dialog.querySelector<HTMLButtonElement>('button[aria-label="重新生成画像"]')!;
    expect(generate.disabled).toBe(true);
    if(!missing)expect(dialog.querySelector<HTMLButtonElement>('[aria-label="移除不确定标签"]')!.disabled).toBe(true);
    await act(async () => generate.click());
    expect(dialog.querySelector(".portrait-template")).toBeNull();
    expect(previewTask).not.toHaveBeenCalled();
    expect(startRun).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("renders the managed paper template fields directly inside the portrait detail without a second dialog", async () => {
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const template:StructuredResource={id:"inline-template",paperId:"paper",kind:"readingTemplate",title:"本文模板",
    sourceUri:"source://library/paper/inline-template",revision:1,
    content:{kind:"reading_template_v1",schemaVersion:1,name:"本文模板",fields:[{id:"question",title:"研究问题",question:"问题定义"}]},
    reviewBaseline:null,pendingFieldIds:[],reviewedFieldIds:[]};
  const portrait:StructuredResource={...template,id:"portrait",kind:"portrait",content:null};
  const gateway={listCandidates:async()=>[]} as unknown as AiAssistanceGateway;
  try{
    const {requestPortraitDetail}=await import("../../../../../../../plugins/ra-literature-library/src/features/ai/portraitDetailNavigation");
    requestPortraitDetail(panelItem().id);
    await act(async()=>root.render(<PaperPortraitPanel item={panelItem()} gateway={gateway} resourceManaged
      managedResource={portrait} paperTemplateResource={template}
      resourceGateway={{getSources:async()=>[]} as unknown as StructuredResourceGateway} onResourceChanged={vi.fn()}/>));
    const detail=document.querySelector('[role="dialog"][aria-label="文献画像详情"]')!;
    expect(detail.querySelector('[aria-label="拖动调整研究问题顺序"]')).not.toBeNull();
    expect(detail.querySelector('[aria-label="编辑研究问题"]')).not.toBeNull();
    expect(document.querySelectorAll('[role="dialog"]')).toHaveLength(1);
    expect(detail.textContent).not.toContain("查看与编辑本文模板");
    expect(detail.querySelector('[aria-label="模板 JSON 草稿"]')).toBeNull();
    const library=detail.querySelector<HTMLButtonElement>('[aria-label="管理全局阅读模板"]')!;
    expect(library.querySelector("svg")).not.toBeNull();
    expect(library.closest("header")).not.toBeNull();
    expect(library.textContent).toBe("");
    expect(detail.textContent).not.toContain("编辑画像分类");
  }finally{await act(async()=>root.unmount());host.remove();}
});
