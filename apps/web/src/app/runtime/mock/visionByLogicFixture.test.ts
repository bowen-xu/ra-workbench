import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { expect, it, vi } from "vitest";
import type { LiteratureItem } from "@research-assistant/core/literature";
import { DEFAULT_PORTRAIT_CATALOG, portraitGenerationGoal, validatePortraitQuotes } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { previewLiteratureFixtures } from "./previewLiteratureFixtures";
import { mockPaperPortraitGateway } from "./mockPaperPortraitGateway";
import { isVisionByLogicAttachment, visionByLogicPortrait, visionByLogicReadingFields } from "./visionByLogicFixture";
import data from "./visionByLogicFixtureData.json";

const mocked = vi.hoisted(() => ({ paper: undefined as LiteratureItem | undefined }));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway", async () => {
  const { previewLiteratureFixtures } = await import("./previewLiteratureFixtures");
  const item = previewLiteratureFixtures.find(value => value.item.attachments.some(attachment => attachment.displayName === "vision-by-logic.pdf"))!.item;
  return { getLiteratureGateway: () => ({ list: async () => [mocked.paper ?? item], get: async () => mocked.paper ?? item }) };
});
const paper = previewLiteratureFixtures.find(value => value.item.attachments.some(isVisionByLogicAttachment))!.item;
const attachment = paper.attachments[0]!;
function storage(): Storage {
  const values = new Map<string, string>();
  return { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear() };
}

it("keeps every fixture quotation grounded in the exact hashed local PDF and page", async () => {
  const bytes = await readFile(new URL("../../../../public/preview/vision-by-logic.pdf", import.meta.url));
  expect(createHash("sha256").update(bytes).digest("hex")).toBe(data.sha256);
  const { getDocument } = await import("pdfjs-dist/legacy/build/pdf.mjs");
  const task = getDocument({ data: new Uint8Array(bytes), useSystemFonts: true, disableFontFace: true });
  const pdf = await task.promise;
  try {
    expect(pdf.numPages).toBe(15);
    for (const source of data.evidence) {
      const content = await (await pdf.getPage(source.pageIndex + 1)).getTextContent();
      const text = content.items.map(item => "str" in item ? item.str : "").join(" ").replace(/\s+/gu, " ").trim();
      expect(text, source.citationKey).toContain(source.quoteExact);
    }
  } finally { await task.destroy(); }
  const candidate = visionByLogicPortrait(paper, attachment);
  const payload = candidate.revisions[0]!.payload;
  if (payload.kind !== "paper_portrait_v1") throw new Error("portrait fixture");
  expect(() => validatePortraitQuotes(payload, candidate.evidenceBundle.items)).not.toThrow();
  expect(payload.claims.some(claim => claim.dimension === "structure")).toBe(false);
  expect(payload.claims.some(claim => claim.dimension === "contribution")).toBe(false);
  expect(payload.claims.every(claim => !/CCN|NAL|MNIST|SVM/u.test(claim.description))).toBe(true);
  expect(payload.template).toHaveLength(9);
  expect(candidate.reviewState).toBe("unreviewed");
  expect(candidate.decisions).toEqual([]);
  expect(isVisionByLogicAttachment({ ...attachment, contentSha256: "a".repeat(64) })).toBe(false);
}, 20_000);

it("seeds once without replacing history and carries all sources into template extraction", async () => {
  const local = storage();
  const historical = { ...visionByLogicPortrait(paper, attachment), id: "old", createdAt: "2026-01-01" };
  local.setItem("portrait-candidates", JSON.stringify([historical]));
  const gateway = mockPaperPortraitGateway(local) as AiAssistanceGateway;
  const candidates = await gateway.listCandidates();
  expect(candidates).toHaveLength(2);
  expect(candidates[1]).toEqual(historical);
  expect(await mockPaperPortraitGateway(local).listCandidates!()).toHaveLength(2);
  const portrait = candidates[0]!;
  const workflow = await gateway.startPaperAnalysis({ clientRequestId: "vision-fixture-test", paperId: paper.id,
    attachmentIds: [attachment.id], overviewProfileSelectionRevision: 0, imageAssets: [],
    paperTemplateRef: { portraitCandidateId: portrait.id, portraitRevision: 1 } });
  const candidate = (await gateway.listCandidates()).find(value => value.id === workflow.turns[0]?.candidateId)!;
  expect(candidate.evidenceBundle.items).toHaveLength(data.evidence.length);
  const payload = candidate.revisions[0]!.payload;
  if (payload.kind !== "paper_overview_unit_v1") throw new Error("overview fixture");
  expect(payload.fields).toHaveLength(9);
  expect(payload.fields.every(field => field.status === "filled")).toBe(true);
  expect(payload.fields.find(field => field.field === "findings")?.statements[0]).toMatchObject({ text: expect.stringContaining("95.5%"), evidenceItemIds: expect.arrayContaining(["E4"]) });
  for (const statement of payload.fields.flatMap(field => field.statements)) for (const quote of statement.evidenceQuotes ?? []) {
    expect(candidate.evidenceBundle.items.find(source => source.citationKey === quote.evidenceItemId)?.quoteExact).toContain(quote.quote);
  }
  expect(visionByLogicReadingFields(["custom"])[0]).toMatchObject({ status: "not_found", statements: [] });
});

it("keeps a failed candidate save out of memory and permits a clean extraction retry", async () => {
  const local = storage();
  const portrait = visionByLogicPortrait(paper, attachment);
  local.setItem("portrait-candidates", JSON.stringify([portrait]));
  local.setItem("vision-portrait-fixture-version", "1");
  const write = local.setItem;
  let full = true;
  local.setItem = (key, value) => {
    if (full && key === "portrait-candidates") throw new DOMException("full", "QuotaExceededError");
    write(key, value);
  };
  const gateway = mockPaperPortraitGateway(local) as AiAssistanceGateway;
  const draft = {clientRequestId: "quota-retry", paperId: paper.id, attachmentIds: [attachment.id],
    overviewProfileSelectionRevision: 0, imageAssets: [],
    paperTemplateRef: {portraitCandidateId: portrait.id, portraitRevision: 1}};
  await expect(gateway.startPaperAnalysis(draft)).rejects.toThrow("full");
  expect(await gateway.listCandidates()).toEqual([portrait]);
  expect(await mockPaperPortraitGateway(local).listCandidates!()).toEqual([portrait]);
  expect(await gateway.getLatestPaperAnalysisWorkflow(paper.id)).toBeUndefined();
  full = false;
  const result = await gateway.startPaperAnalysis(draft);
  expect(result.status).toBe("needs_review");
  expect(await gateway.listCandidates()).toHaveLength(2);
});

it("regenerates a portrait with published resource UUIDs while retaining previous candidates", async () => {
  const local = storage();
  const gateway = mockPaperPortraitGateway(local) as AiAssistanceGateway;
  const before = await gateway.listCandidates();
  const catalog = { ...DEFAULT_PORTRAIT_CATALOG, templates: DEFAULT_PORTRAIT_CATALOG.templates.map((template, index) => ({
    ...template, id: `12345678-1234-4234-8234-${String(index).padStart(12, "0")}`,
  })) };
  const draft = { taskKind: "paper_profile_classification" as const, providerId: "fixture-generation" as const,
    userGoal: portraitGenerationGoal(catalog), pipelineVersion: "pages-v1", promptVersion: "catalog-v2", outputSchemaVersion: 1,
    maxInputTokens: 24_000, maxOutputTokens: 8000, timeoutMs: 120_000,
    scopeItems: [{ itemKind: "attachment" as const, objectId: attachment.id, parentObjectId: paper.id, contentSha256: attachment.contentSha256 }] };
  const preview = await gateway.previewTask(draft);
  const run = await gateway.startRun({ ...draft, confirmedScopeHash: preview.scopeHash });
  expect((await gateway.getRun(run.id)).status).toBe("needs_review");
  const after = await gateway.listCandidates();
  expect(after).toHaveLength(before.length + 1);
  expect(after.slice(1)).toEqual(before);
  const generated = after[0]!;
  expect(generated.runId).toBe(run.id);
  if (generated.revisions[0]?.payload.kind !== "paper_portrait_v1") throw new Error("portrait");
  validatePortraitQuotes(generated.revisions[0].payload, generated.evidenceBundle.items);
  expect(await mockPaperPortraitGateway(local).listCandidates!()).toEqual(after);
});


it("freezes multiple selected attachments and remaps literal reading evidence when Vision is not first", async () => {
  const supplement = { ...attachment, id: "supplement", displayName: "supplement.pdf", contentSha256: "f".repeat(64) };
  mocked.paper = { ...paper, attachments: [supplement, attachment] };
  try {
    const local = storage(); local.setItem("vision-portrait-fixture-version", "1");
    const gateway = mockPaperPortraitGateway(local) as AiAssistanceGateway;
    const draft = { taskKind: "paper_profile_classification" as const, providerId: "fixture-generation" as const, userGoal: portraitGenerationGoal(DEFAULT_PORTRAIT_CATALOG), pipelineVersion: "pages-v1", promptVersion: "catalog-v2", outputSchemaVersion: 1, maxInputTokens: 24_000, maxOutputTokens: 8000, timeoutMs: 120_000,
      scopeItems: mocked.paper.attachments.map(value => ({ itemKind: "attachment" as const, objectId: value.id, parentObjectId: paper.id, contentSha256: value.contentSha256 })) };
    const preview = await gateway.previewTask(draft);
    expect(preview.evidencePreview).toEqual([]);
    expect(preview.inputItemCount).toBe(2);
    await gateway.startRun({ ...draft, confirmedScopeHash: preview.scopeHash });
    const portrait = (await gateway.listCandidates())[0]!;
    expect(portrait.evidenceBundle.items).toHaveLength(11);
    expect(new Set(portrait.evidenceBundle.items.map(value => value.citationKey)).size).toBe(11);
    expect(portrait.evidenceBundle.items[0]?.attachmentId).toBe("supplement");
    expect(portrait.revisions[0]?.payload).toMatchObject({ template: DEFAULT_PORTRAIT_CATALOG.templates.find(value => value.id === "formal")!.fields });
    if (portrait.revisions[0]!.payload.kind !== "paper_portrait_v1") throw new Error("portrait");
    validatePortraitQuotes(portrait.revisions[0]!.payload, portrait.evidenceBundle.items);
    const workflow = await gateway.startPaperAnalysis({ clientRequestId: "multi-portrait", paperId: paper.id, attachmentIds: [supplement.id, attachment.id], overviewProfileSelectionRevision: 0, imageAssets: [], paperTemplateRef: { portraitCandidateId: portrait.id, portraitRevision: 1 } });
    const extraction = (await gateway.listCandidates()).find(value => value.id === workflow.turns[0]?.candidateId)!;
    if (extraction.revisions[0]?.payload.kind !== "paper_overview_unit_v1") throw new Error("overview");
    for (const statement of extraction.revisions[0].payload.fields.flatMap(field => field.statements)) for (const quote of statement.evidenceQuotes ?? []) {
      expect(extraction.evidenceBundle.items.find(source => source.citationKey === quote.evidenceItemId)?.quoteExact).toContain(quote.quote);
    }
    expect(extraction.revisions[0].payload.fields.find(field => field.field === "properties")?.status).toBe("not_found");
  } finally { mocked.paper = undefined; }
});
