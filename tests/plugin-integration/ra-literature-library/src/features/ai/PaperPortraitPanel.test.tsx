// @vitest-environment happy-dom
import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_PORTRAIT_TEMPLATE, PORTRAIT_DIMENSIONS, type AiCandidate, type PaperPortraitCandidateV1 } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { PaperPortraitPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperPortraitPanel";
import { selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import type { StructuredResourceGateway } from "../../../../../../apps/web/src/core/services/structuredResourceGateway";
import { loadPortraitCatalog } from "../../../../../../../plugins/ra-literature-library/src/features/ai/portraitCatalogPreferences";

const notices = vi.hoisted(() => ({ success: vi.fn(), info: vi.fn(), warning: vi.fn(), error: vi.fn() }));
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: notices }));

const paper: LiteratureItem = {
  id: "paper", itemType: "journalArticle", title: "Paper", abstractText: "", containerTitle: "", volume: "", issue: "", pages: "", doi: "", url: "", language: "", bibtexKey: "", creators: [], tags: [], readingStatus: "unread",
  attachments: [{ id: "attachment", displayName: "paper.pdf", mimeType: "application/pdf", byteSize: 10, contentSha256: "a".repeat(64) }], projectIds: [], metadataSources: [], createdAt: "now", updatedAt: "now",
};
function fixture(): AiCandidate {
  const payload: PaperPortraitCandidateV1 = { kind: "paper_portrait_v1", schemaVersion: 1, coverage: "仅第1页", claims: PORTRAIT_DIMENSIONS.map((dimension, index) => ({ id: `claim_${index}`, dimension, label: `标签${index}`, description: "根据摘录可见显式模型。", certainty: "supported", evidence: [{ evidenceItemId: "E1", quote: "explicit model" }] })), template: DEFAULT_PORTRAIT_TEMPLATE };
  return {
    id: "candidate", runId: "run", providerId: "provider", modelId: "model", candidateKind: "paper_portrait_v1", schemaVersion: 1, currentRevision: 1, reviewState: "unreviewed", validationStatus: "valid", originalPayloadHash: "b".repeat(64), createdAt: "now", updatedAt: "now",
    evidenceBundle: { id: "bundle", runId: "run", scopeHash: "c".repeat(64), retrievalConfigVersion: "portrait-v1", validationStatus: "valid", createdAt: "now", items: [{ id: "evidence", citationKey: "E1", ordinal: 0, anchorId: "anchor", paperId: paper.id, paperTitle: paper.title, attachmentId: "attachment", contentSha256: "a".repeat(64), pageIndex: 0, quoteExact: "An explicit model is presented.", quotePrefix: "", quoteSuffix: "", contextText: "An explicit model is presented.", relationKind: "direct_quote", validationStatus: "valid", currentAnchorStatus: "page_level", createdAt: "now" }] },
    revisions: [{ id: "revision", revision: 1, payloadHash: "d".repeat(64), editorKind: "ai", sourceRunId: "run", changeReason: "generated", createdAt: "now", payload }],
    decisions: [], attentionFeedback: [], citationAssessments: [], suggestionItemDecisions: [], overviewFieldConfirmations: [], inspectedEvidenceItemIds: [],
  };
}

describe("paper portrait production panel", () => {
  let container: HTMLDivElement;
  let root: Root;
  let candidate: AiCandidate;
  let gateway: AiAssistanceGateway;
  let currentPortrait:StructuredResource;
  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div"); document.body.append(container); root = createRoot(container);
    candidate = fixture();
    gateway = { listCandidates: vi.fn(async () => [candidate]), recordEvidenceInspection: vi.fn(async () => true), reviseCandidate: vi.fn(async draft => {
      candidate = { ...candidate, currentRevision: 2, reviewState: "edited", revisions: [...candidate.revisions, { ...candidate.revisions[0]!, id: "revision2", revision: 2, payload: draft.payload, editorKind: "human" }] }; return candidate;
    }) } as unknown as AiAssistanceGateway;
    const values = new Map<string, string>();
    selectAppStorage({ get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
      getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); }, clear: () => values.clear() });
    vi.clearAllMocks();
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  async function render(open = true): Promise<void> {
    const records=await gateway.listCandidates(undefined,{paperId:paper.id});
    currentPortrait={id:"portrait",paperId:paper.id,kind:"portrait",sourceUri:"source://library/paper/portrait",title:"文献画像",revision:1,content:records[0]?.revisions[0]?.payload??null,reviewBaseline:records[0]?.revisions[0]?.payload??null,pendingFieldIds:[],reviewedFieldIds:[]};
    const resources={write:vi.fn(async(input:{content:string})=>{currentPortrait={...currentPortrait,revision:currentPortrait.revision+1,content:JSON.parse(input.content),reviewBaseline:JSON.parse(input.content)};return {status:"applied",resource:currentPortrait,errors:[]};})} as unknown as StructuredResourceGateway;
    function Harness(){const [resource,setResource]=useState(currentPortrait);return <PaperPortraitPanel item={paper} gateway={gateway} resourceManaged managedResource={resource} resourceGateway={resources} onResourceChanged={setResource}/>;}
    await act(async()=>root.render(createElement(Harness)));
    if(open)await act(async()=>button("查看画像详情").click());
  }
  function button(text: string): HTMLButtonElement { const result = [...document.body.querySelectorAll("button")].find(value => (value.textContent === text || value.getAttribute("aria-label") === text) && !value.closest('[hidden]')); if (!result) throw new Error(`Missing ${text}`); return result; }
  async function input(selector: string, value: string): Promise<void> {
    const element = document.body.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector); if (!element) throw new Error(selector);
    await act(async () => { Object.getOwnPropertyDescriptor(element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype, "value")?.set?.call(element, value); element.dispatchEvent(new Event("input", { bubbles: true })); });
  }
  it("shows a compact read-only summary and opens all editing controls in a floating detail panel", async () => {
    await render(false);
    expect(container.querySelectorAll("dl > div")).toHaveLength(3);
    expect(container.querySelector("input, textarea, select")).toBeNull();
    expect(container.querySelectorAll("button")).toHaveLength(1);
    expect(container.textContent).not.toContain("根据摘录可见显式模型");
    await act(async () => button("查看画像详情").click());
    expect(document.querySelector('[role="dialog"][aria-label="文献画像详情"]')).not.toBeNull();
    expect(document.querySelector('[aria-label="画像版本"]')).toBeNull();
    expect(document.body.textContent).not.toContain("采纳画像与模板");
    await act(async()=>button("关闭画像详情").click());
    expect(document.querySelector('[role="dialog"][aria-label="文献画像详情"]')).toBeNull();
  });


  it("shows three classification dimensions and preserves hidden legacy contributions and structure", async () => {
    const payload = candidate.revisions[0]!.payload;
    if (payload.kind !== "paper_portrait_v1") throw new Error("fixture");
    const legacy = { ...payload, claims: [...payload.claims, { ...payload.claims[0]!, id: "old_structure", dimension: "structure" as const, label: "IMRaD" }, { ...payload.claims[0]!, id: "old_contribution", dimension: "contribution" as const, label: "Old contribution" }] };
    candidate = { ...candidate, providerId: "fixture-generation", revisions: [{ ...candidate.revisions[0]!, payload: legacy }] };
    await render();
    expect(document.querySelectorAll(".portrait-dimension")).toHaveLength(3);
    expect(document.querySelectorAll(".paper-portrait-summary dl > div")).toHaveLength(3);
    expect(document.body.textContent).not.toContain("模拟样例");
    expect(document.body.textContent).not.toContain("核心贡献");
    expect(document.body.textContent).not.toContain("组织结构");
    expect(document.body.querySelector("button[data-portrait-source]")).toBeNull();
    expect(document.body.textContent).not.toContain("根据摘录可见显式模型");
    expect(document.body.textContent).not.toContain("判断状态");
    expect(candidate.revisions[0]!.payload).toEqual(legacy);
  });
  it("makes the saved unreviewed model template available without making a human decision", async () => {
    const changed = vi.fn();
    await act(async () => root.render(createElement(PaperPortraitPanel, { item: paper, gateway, onConfirmedTemplateChanged: changed })));
    expect(changed).toHaveBeenLastCalledWith(expect.objectContaining({ portraitCandidateId: candidate.id, fields: DEFAULT_PORTRAIT_TEMPLATE }), "unconfirmed");
    expect(candidate.reviewState).toBe("unreviewed");
    expect(candidate.decisions).toEqual([]);
  });




  it("configures classification tags and selects them without arbitrary label or certainty inputs", async () => {
    await render();
    expect(document.querySelector('input[aria-label="修改文献类型标签"]')).toBeNull();
    await act(async () => button("编辑研究范式标签").click());
    await act(async () => button("新建标签").click());
    await input('input[aria-label="标签名称"]', "机制比较");
    await input('textarea[aria-label="标签解释说明"]', "比较不同机制及其解释能力。" );
    await act(async () => button("保存标签").click());
    expect(loadPortraitCatalog().tags.paradigm.at(-1)?.label).toBe("机制比较");
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="添加研究范式标签"]')!.click());
    await act(async () => button("机制比较").click());
    expect(document.querySelector('[aria-label="研究范式"] .portrait-selected-tag')?.parentElement?.parentElement?.textContent).toContain("机制比较");
    expect(currentPortrait.content).toEqual(expect.objectContaining({ claims: expect.arrayContaining([expect.objectContaining({ dimension: "paradigm", label: "机制比较", certainty: "uncertain", evidence: [] })]) }));
  });
  it("freezes all selected PDF attachments and configured definitions in generation", async () => {
    gateway = { ...gateway, previewTask: vi.fn(async () => { throw new Error("controlled stop"); }) };
    const multi = { ...paper, attachments: [...paper.attachments, { ...paper.attachments[0]!, id: "supplement", displayName: "supplement.pdf", contentSha256: "f".repeat(64) }] };
    await act(async () => root.render(createElement(PaperPortraitPanel, { item: multi, gateway })));
    await act(async () => button("查看画像详情").click());
    expect(document.querySelector('.portrait-source-options input')).toBeNull();
    expect(document.querySelector('.portrait-source-files')?.textContent).not.toContain("supplement.pdf");
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="添加来源 PDF"]')!.click());
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>('.portrait-source-menu button')].find(value => value.textContent === "supplement.pdf")!.click());
    expect(document.querySelectorAll('.portrait-source-chip')).toHaveLength(2);
    await act(async () => button("重新生成画像").click());
    expect(gateway.previewTask).toHaveBeenCalledWith(expect.objectContaining({ scopeItems: [expect.objectContaining({ objectId: "attachment" }), expect.objectContaining({ objectId: "supplement" })], userGoal: expect.stringContaining("分类标签库及阅读模板：") }));
    expect(document.querySelector('.portrait-detail-heading')?.textContent).not.toContain(paper.title);
    expect(document.querySelector('.portrait-paper-title')?.textContent).toBe(paper.title);
  });
  it("adds and removes source chips without presenting a second generation step", async () => {
    gateway = { ...gateway, previewTask: vi.fn(async () => ({ evidencePreview: [{ citationKey: "E1", pageIndex: 0, quoteExact: "text" }], requiresCloudConsent: false, providerId: "local", modelId: "model", scopeHash: "scope" })) } as unknown as AiAssistanceGateway;
    await render();
    expect(document.querySelectorAll('.portrait-source-chip')).toHaveLength(1);
    expect(document.querySelector('.portrait-source-options input[type="checkbox"]')).toBeNull();
    await act(async () => button("重新生成画像").click());
    expect(document.querySelector('.portrait-scope')).toBeNull();
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="移除来源 paper.pdf"]')!.click());
    expect(document.querySelector('.portrait-scope')).toBeNull();
    expect(document.querySelector('.portrait-source-chip')).toBeNull();
    expect(button("重新生成画像").disabled).toBe(true);
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="添加来源 PDF"]')!.click());
    await act(async () => document.querySelector<HTMLButtonElement>('.portrait-source-menu button')!.click());
    expect(document.querySelector('.portrait-source-chip')?.textContent).toContain("paper.pdf");
    expect(button("重新生成画像").disabled).toBe(false);
  });


  it("selects one tag from the category menu and edits its definition in a floating dialog", async () => {
    const payload = candidate.revisions[0]!.payload;
    if (payload.kind !== "paper_portrait_v1") throw new Error("fixture");
    candidate = { ...candidate, revisions: [{ ...candidate.revisions[0]!, payload: { ...payload, claims: payload.claims.map(claim => claim.dimension === "genre" ? { ...claim, label: "原创研究", description: "本文实现 CCN 并评测 MNIST。" } : claim) } }] };
    await render();
    const label = document.querySelector<HTMLLabelElement>('[aria-label="文献类型"] .portrait-classification-description')!;
    expect(label.textContent).toContain("提出新的研究问题、方法或发现");
    expect(label.textContent).not.toContain("CCN");
    await act(async () => label.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(label.hasAttribute("tabindex")).toBe(false);
    expect(document.querySelector('.portrait-tag-dialog, .portrait-classification-row textarea')).toBeNull();
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="编辑文献类型标签"]')!.click());
    expect(document.querySelector('.portrait-tag-dialog')).toBeNull();
    expect(document.querySelectorAll('[role="menu"][aria-label="编辑文献类型标签"] button')).toHaveLength(loadPortraitCatalog().tags.genre.length + 1);
    await act(async () => button("原创研究").click());
    expect(document.querySelectorAll('.portrait-tag-dialog input')).toHaveLength(1);
    expect(document.querySelector('.portrait-dimension input')).toBeNull();
    await input('textarea[aria-label="标签解释说明"]', "提出新知识并提供相应论证或证据。");
    await act(async () => button("保存标签").click());
    expect(loadPortraitCatalog().tags.genre.find(tag => tag.label === "原创研究")?.description).toBe("提出新知识并提供相应论证或证据。");
    expect(document.querySelector('.portrait-classification-description')?.textContent).toBe("提出新知识并提供相应论证或证据。");
    expect(document.querySelector('.portrait-tag-dialog')).toBeNull();
    expect(document.body.textContent).not.toContain("保存修改");
    expect(candidate.revisions[0]!.payload).toMatchObject({ claims: expect.arrayContaining([expect.objectContaining({ description: "本文实现 CCN 并评测 MNIST。" })]) });
    expect(gateway.reviseCandidate).not.toHaveBeenCalled();
  });

  it("keeps add and edit controls beside each category title and descriptions read-only", async () => {
    await render();
    expect(document.querySelector('[aria-label="文献类型"]')?.textContent).not.toContain("综述");
    const label = document.querySelector<HTMLLabelElement>('[aria-label="文献类型"] .portrait-classification-description')!;
    expect(document.querySelector('[aria-label="文献类型"] textarea')).toBeNull();
    await act(async () => label.dispatchEvent(new MouseEvent("dblclick", { bubbles: true })));
    expect(document.querySelector('[aria-label="文献类型"] textarea')).toBeNull();
    for (const title of ["文献类型", "知识形态", "研究范式"]) {
      const row = document.querySelector(`[aria-label="${title}"] .setting-row`)!;
      expect(row.querySelector(".setting-title")?.textContent).toBe(title);
      expect(row.querySelector(`.setting-control button[aria-label="添加${title}标签"]`)).not.toBeNull();
      expect(row.querySelector(`.setting-control button[aria-label="编辑${title}标签"]`)).not.toBeNull();
    }
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="添加文献类型标签"]')!.click());
    await act(async () => button("原创研究").click());
    expect(document.querySelector('[aria-label="文献类型"] .portrait-classification-list')?.textContent).toContain("原创研究");
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="移除原创研究标签"]')!.click());
    expect(document.querySelector('[aria-label="文献类型"] .portrait-classification-list')?.textContent).not.toContain("原创研究");
    expect(document.querySelector('.portrait-contribution-list blockquote')).toBeNull();
  });
  it("cancels category edits without changing the catalog or selected paper tags", async () => {
    await render();
    const before = loadPortraitCatalog();
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="编辑知识形态标签"]')!.click());
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="menu"][aria-label="编辑知识形态标签"] button')].find(value => value.textContent === "概念分析")!.click());
    await input('input[aria-label="标签名称"]', "未保存的名称");
    await input('textarea[aria-label="标签解释说明"]', "未保存的说明");
    await act(async () => button("取消").click());
    expect(loadPortraitCatalog()).toEqual(before);
    expect(document.querySelector('.portrait-tag-dialog')).toBeNull();
    expect(gateway.reviseCandidate).not.toHaveBeenCalled();
  });
  it("renames a tag without altering other categories or the paper's selected labels", async () => {
    await render();
    const before = loadPortraitCatalog();
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="编辑知识形态标签"]')!.click());
    await act(async () => button("形式描述").click());
    await input('input[aria-label="标签名称"]', "形式化描述");
    await act(async () => button("保存标签").click());
    expect(loadPortraitCatalog().tags.mode.find(tag => tag.label === "形式化描述")).toBeDefined();
    expect(loadPortraitCatalog().tags.genre).toEqual(before.tags.genre);
    expect(loadPortraitCatalog().tags.paradigm).toEqual(before.tags.paradigm);
    expect(document.querySelector('.portrait-selected-tag')?.textContent).toBe("标签0");
    expect(gateway.reviseCandidate).not.toHaveBeenCalled();
  });
  it("keeps invalid or failed tag edits in the dialog for correction", async () => {
    await render();
    const before = loadPortraitCatalog();
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="编辑研究范式标签"]')!.click());
    await act(async () => button("受控实验").click());
    await input('input[aria-label="标签名称"]', "观察研究");
    await act(async () => button("保存标签").click());
    expect(notices.error).toHaveBeenCalledWith(expect.stringContaining("同名标签"));
    expect(document.querySelector('.portrait-tag-dialog')).not.toBeNull();
    expect(loadPortraitCatalog()).toEqual(before);
    await input('input[aria-label="标签名称"]', "受控实验");
    selectAppStorage({ length: 0, key: () => null, getItem: () => null, setItem: () => { throw new Error("storage unavailable"); }, removeItem: () => undefined, clear: () => undefined });
    await input('textarea[aria-label="标签解释说明"]', "保存失败仍保留的定义。");
    await act(async () => button("保存标签").click());
    expect(document.querySelector<HTMLTextAreaElement>('.portrait-tag-dialog textarea')?.value).toBe("保存失败仍保留的定义。");
    expect(notices.error).toHaveBeenCalled();
  });
  it("closes a single-tag editor on Escape while retaining the portrait panel", async () => {
    await render();
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="编辑文献类型标签"]')!.click());
    await act(async () => button("原创研究").click());
    await act(async () => {
      const input = document.querySelector<HTMLInputElement>('.portrait-tag-dialog input')!;
      input.focus(); input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    });
    expect(document.querySelector('.portrait-tag-dialog')).toBeNull();
    expect(document.querySelector('[role="dialog"][aria-label="文献画像详情"]')).not.toBeNull();
  });


  it("creates a custom dimension with stable identity, saves its taxonomy and includes it in the next task", async () => {
    gateway = { ...gateway, previewTask: vi.fn(async () => { throw new Error("controlled stop"); }) };
    await render();
    expect(document.querySelector('button[aria-label="配置标签"]')).toBeNull();
    await act(async () => button("新增画像维度").click());
    await input('input[aria-label="维度名称"]', "开放程度");
    await input('textarea[aria-label="维度说明"]', "研究材料可获取与复用的程度。");
    await act(async () => button("创建维度").click());
    const dimension = loadPortraitCatalog().customDimensions?.[0]!;
    expect(dimension.id).toMatch(/^custom_/);
    expect(document.querySelector('section[aria-label="开放程度"]')).not.toBeNull();
    await act(async () => button("编辑开放程度标签").click());
    await act(async () => button("新建标签").click());
    await input('input[aria-label="标签名称"]', "开放数据");
    await input('textarea[aria-label="标签解释说明"]', "公开提供可复用数据。");
    await act(async () => button("保存标签").click());
    await act(async () => button("添加开放程度标签").click());
    await act(async () => button("开放数据").click());
    expect(currentPortrait.content).toEqual(expect.objectContaining({ customDimensions: [dimension], claims: expect.arrayContaining([expect.objectContaining({ dimension: dimension.id, label: "开放数据", evidence: [] })]) }));
    await act(async () => button("重新生成画像").click());
    expect(loadPortraitCatalog().customDimensions).toEqual(expect.arrayContaining([dimension]));
  });
  it("offers generation in an empty summary and hides it once a portrait exists", async () => {
    gateway = { ...gateway, listCandidates: vi.fn(async () => []), previewTask: vi.fn(async () => { throw new Error("controlled stop"); }) };
    await render(false);
    expect(container.querySelector('button[aria-label="生成画像"]')).not.toBeNull();
    await act(async () => button("生成画像").click());
    expect(container.querySelectorAll('.paper-portrait-summary [aria-label="生成画像"]')).toHaveLength(1);
    expect(document.querySelector('[aria-label="文献画像详情"]')).toBeNull();
  });


});
