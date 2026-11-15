// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { AiCandidate, AiTaskSpecDraft } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { mockPaperPortraitGateway } from "./mockPaperPortraitGateway";
import { mockPaperTemplateGateway } from "./mockPaperTemplateGateway";
import { browserKnowledgeStorage } from "../../../../../../plugins/literature_library/src/services/browserKnowledgeStorage";
import { browserKnowledgeGateway } from "../../../../../../plugins/literature_library/src/services/browserKnowledgeGateway";
import { saveBrowserPaperProfileSelection } from "../../../../../../plugins/literature_library/src/services/browserReadingProfiles";
import { selectAppStorage, getAppStorage } from "../../../core/runtime/appStorage";
import { OverviewFieldSources } from "../../../../../../plugins/literature_library/src/features/ai/OverviewFieldSources";
import { TemplateReadingRecords } from "../../../../../../plugins/literature_library/src/features/ai/TemplateReadingRecords";
import type { AiSourceInspectionRequest } from "../../../../../../plugins/literature_library/src/features/ai/aiEvidenceNavigation";

vi.mock("@research-assistant/literature-library-plugin/literatureGateway", () => ({ getLiteratureGateway: () => ({
  list: async () => [{ id: "paper", title: "Synthetic Research Preview", attachments: [{ id: "attachment", displayName: "sample.pdf", mimeType: "application/pdf", contentSha256: "a".repeat(64) }] }],
  get: async () => ({ id: "paper", title: "Synthetic Research Preview", attachments: [{ id: "attachment", displayName: "sample.pdf", mimeType: "application/pdf", contentSha256: "a".repeat(64) }] }),
}) }));
const previousStorage = getAppStorage();
const values = new Map<string, string>();
const storage: Storage = { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
  getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); }, clear: () => values.clear() };
afterEach(() => { storage.clear(); selectAppStorage(previousStorage); });

it("extracts an explicit profile without a portrait, freezes its fields and supports confirmation and undo", async () => {
  selectAppStorage(storage);
  const gateway = mockPaperPortraitGateway(storage) as AiAssistanceGateway;
  const draft = { clientRequestId: "profiles", paperId: "paper", attachmentIds: ["attachment"], overviewProfileSelectionRevision: 0, imageAssets: [] };
  await expect(gateway.startPaperAnalysis(draft)).rejects.toThrow("选择阅读框架");
  const selection = saveBrowserPaperProfileSelection({ paperId: "paper", expectedRevision: 0,
    profiles: [{ profileId: "generic_overview", profileVersion: 1 }, { profileId: "philosophical_conceptual", profileVersion: 1 }] });
  await expect(gateway.startPaperAnalysis(draft)).rejects.toThrow("已变化");
  const request = { ...draft, overviewProfileSelectionRevision: selection.revision };
  const workflow = await gateway.startPaperAnalysis(request);
  const candidate = (await gateway.listCandidates()).find(value => value.id === workflow.turns[0]?.candidateId)!;
  const payload = candidate.revisions[0]!.payload;
  if (payload.kind !== "paper_overview_unit_v1") throw new Error("fixture");
  expect(payload.profileRefs).toEqual(selection.profiles);
  expect(payload.paperTemplate).toBeUndefined();
  expect(payload.fields.map(field => field.field)).toEqual(["research_question", "approach", "key_result", "conclusion", "limitation", "open_question", "argument", "objection", "response"]);
  await expect(gateway.startPaperAnalysis(request)).rejects.toThrow("已经存在");
  await expect(gateway.reviseCandidate({ candidateId: candidate.id, expectedRevision: 1, payload: { ...payload, profileRefs: [...selection.profiles].reverse() }, changeReason: "change frame" })).rejects.toThrow("冻结");
  const source = candidate.evidenceBundle.items[0]!;
  await gateway.recordEvidenceInspection(candidate.id, 1, source.id);
  await gateway.recordCitationAssessment({ candidateId: candidate.id, candidateRevision: 1, statementId: "mock_statement", evidenceItemId: source.id,
    expectedAssessmentRevision: 0, semanticRelation: "supports", rationale: "fixture", idempotencyKey: "review-profile" });
  const result = await gateway.confirmOverviewFields({ candidateId: candidate.id, candidateRevision: 1, fieldKinds: ["research_question"], scopeKind: "field", idempotencyKey: "confirm-profile" });
  expect(browserKnowledgeStorage.readReadingUnits()[0]).toMatchObject({ unitKind: "research_question", paperId: "paper", provenanceKind: "ai_accepted_unchanged" });
  expect(browserKnowledgeStorage.readReadingUnits()[0]?.templateField).toBeUndefined();
  await gateway.undoOverviewConfirmation({ batchId: result.batch.id, idempotencyKey: "undo-profile" });
  expect(browserKnowledgeStorage.readReadingUnits()).toEqual([]);
});

it.each([true, false])("uses a template (optional adoption: %s) through extraction, source selection, confirmation and safe undo", async adopt => {
  selectAppStorage(storage);
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const gateway = mockPaperPortraitGateway(storage) as AiAssistanceGateway;
  const task: AiTaskSpecDraft = { taskKind: "paper_profile_classification", providerId: "fixture-generation", userGoal: "画像",
    pipelineVersion: "v1", promptVersion: "v1", outputSchemaVersion: 1, maxInputTokens: 2000, maxOutputTokens: 2000, timeoutMs: 5000,
    scopeItems: [{ itemKind: "attachment", objectId: "attachment", parentObjectId: "paper" }] };
  const preview = await gateway.previewTask(task);
  await gateway.startRun({ ...task, confirmedScopeHash: preview.scopeHash });
  let portrait = (await gateway.listCandidates())[0]!;
  const draft = { clientRequestId: "template-test", paperId: "paper", attachmentIds: ["attachment"], overviewProfileSelectionRevision: 0,
    paperTemplateRef: { portraitCandidateId: portrait.id, portraitRevision: 1 }, imageAssets: [] };
  await expect(gateway.startPaperAnalysis({ ...draft, paperTemplateRef: { ...draft.paperTemplateRef, portraitCandidateId: "missing" } })).rejects.toThrow("不可用");
  const source = portrait.evidenceBundle.items[0]!;
  const portraitPayload = portrait.revisions[0]!.payload;
  if (portraitPayload.kind !== "paper_portrait_v1") throw new Error("fixture");
  if (adopt) portrait = await gateway.decideCandidate({ candidateId: portrait.id, candidateRevision: 1, action: "accept_unchanged", rationale: "fixture", idempotencyKey: "accept" });
  const workflow = await gateway.startPaperAnalysis(draft);
  expect(workflow.status).toBe("needs_review");
  expect(workflow.turns.map(turn => turn.kind)).toEqual(["paper_overview"]);
  expect(portrait.reviewState).toBe(adopt ? "accepted_unchanged" : "unreviewed");
  expect(portrait.inspectedEvidenceItemIds).toEqual([]);
  expect(portrait.citationAssessments).toEqual([]);
  let candidate = (await gateway.listCandidates()).find(value => value.candidateKind === "paper_overview_unit_v1")!;
  const payload = candidate.revisions[0]!.payload;
  if (payload.kind !== "paper_overview_unit_v1") throw new Error("fixture");
  expect(payload.paperTemplate?.fields).toEqual(portraitPayload.template);
  expect(payload.fields.slice(1).every(field => field.status === "not_found")).toBe(true);
  candidate = await gateway.reviseCandidate({ candidateId: candidate.id, expectedRevision: 1, changeReason: "human_quote_review",
    payload: { ...payload, fields: payload.fields.map((field, index) => index > 0 ? field : { ...field,
      statements: field.statements.map(statement => ({ ...statement, evidenceQuotes: [{ evidenceItemId: source.citationKey, quote: "Research Preview" }] })) }) } });
  const current = candidate.revisions.at(-1)!.payload;
  if (current.kind !== "paper_overview_unit_v1") throw new Error("fixture");
  const inspect = vi.fn((event: Event) => {
    event.preventDefault(); const request = (event as CustomEvent<AiSourceInspectionRequest>).detail;
    expect(request.source.quoteExact).toBe("Research Preview"); expect(request.source.quotePrefix).toBe("Synthetic "); request.resolve();
  });
  window.addEventListener("research-assistant:inspect-reading-source", inspect);
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  try {
    await act(async () => root.render(<OverviewFieldSources candidate={candidate} field={current.fields[0]!} gateway={gateway} onChanged={() => undefined} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="跳转到原文"]')?.click());
    expect(inspect).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); container.remove(); window.removeEventListener("research-assistant:inspect-reading-source", inspect); }
  const confirmation = { candidateId: candidate.id, candidateRevision: 2, fieldKinds: [current.fields[0]!.field], scopeKind: "field" as const, idempotencyKey: "confirm" };
  const result = await gateway.confirmOverviewFields(confirmation);
  let unit = browserKnowledgeStorage.readReadingUnits()[0]!;
  expect(unit.templateField?.fieldId).toBe(current.fields[0]!.field);
  expect(result.candidate.overviewFieldConfirmations[0]?.active).toBe(true);
  expect(result.candidate.citationAssessments).toEqual([]);
  expect(result.candidate.reviewState).toBe("accepted_after_edit");
  expect(unit.provenanceKind).toBe("ai_accepted_after_edit");
  expect(result.batch.fieldConfirmations[0]?.decisionKind).toBe("human_edited");
  await expect(gateway.startPaperAnalysis({ ...draft, clientRequestId: "duplicate-test" })).rejects.toThrow("已经存在");
  // A separately generated candidate may update the same formal field; it must retain undo history.
  const next: AiCandidate = { ...candidate, id: "next", currentRevision: 1, reviewState: "unreviewed", overviewFieldConfirmations: [],
    inspectedEvidenceItemIds: [source.id], revisions: [candidate.revisions[0]!],
    citationAssessments: result.candidate.citationAssessments.map(value => ({ ...value, candidateId: "next", candidateRevision: 1 })) };
  let stored = [result.candidate, next];
  const replacementGateway = mockPaperTemplateGateway(storage, { list: () => stored, add: value => { stored.push(value); },
    update: value => { stored = stored.map(candidate => candidate.id === value.id ? value : candidate); return value; } });
  const nextResult = await replacementGateway.confirmOverviewFields!({ ...confirmation, candidateId: next.id, candidateRevision: 1, idempotencyKey: "next-confirm" });
  const updated = browserKnowledgeStorage.readReadingUnits()[0]!;
  expect(updated.id).toBe(unit.id); expect(updated.revision).toBe(2);
  expect(browserKnowledgeStorage.readReadingUnits()).toHaveLength(1);
  const recordsRoot = createRoot(container); document.body.append(container);
  try {
    await act(async () => recordsRoot.render(<TemplateReadingRecords template={payload.paperTemplate} units={[updated]}
      gateway={browserKnowledgeGateway} onSaved={async () => undefined} aiGateway={gateway} candidates={[result.candidate, nextResult.candidate]} />));
    expect(container.querySelector('[aria-label="跳转到原文"]')).toBeNull();
    expect(container.querySelector(".reading-field-sources button")).not.toBeNull();
  } finally { await act(async () => recordsRoot.unmount()); container.remove(); }
  await replacementGateway.undoOverviewConfirmation!({ batchId: nextResult.batch.id, idempotencyKey: "undo-update" });
  unit = browserKnowledgeStorage.readReadingUnits()[0]!;
  expect(unit.revision).toBe(3); expect(unit.provenanceKind).toBe("ai_accepted_after_edit");
  const edited = await browserKnowledgeGateway.saveReadingUnit({ id: unit.id, expectedRevision: unit.revision, paperId: "paper", unitKind: "other", title: unit.title, body: "Later manual text", epistemicRole: "working_note" });
  expect(edited.templateField).toEqual(unit.templateField);
  await expect(gateway.undoOverviewConfirmation({ batchId: result.batch.id, idempotencyKey: "undo" })).rejects.toThrow("后续修改");
  expect(browserKnowledgeStorage.readReadingUnits()[0]?.body).toBe("Later manual text");
});
