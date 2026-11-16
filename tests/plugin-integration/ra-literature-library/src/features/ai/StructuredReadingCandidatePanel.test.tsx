// @vitest-environment happy-dom

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { AiCandidate, AiOverviewConfirmationResult } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const notifications = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() }));
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: notifications }));

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { StructuredReadingCandidatePanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/StructuredReadingCandidatePanel";
import { fieldReadingSources, latestReadingSources, readingFieldColor } from "../../../../../../../plugins/ra-literature-library/src/features/ai/readingFieldSources";
import type { AiSourceInspectionRequest } from "../../../../../../../plugins/ra-literature-library/src/features/ai/aiEvidenceNavigation";

function candidate({ citationReady = false }: { readonly citationReady?: boolean } = {}): AiCandidate {
  return {
    id: "candidate", runId: "run", providerId: "provider", modelId: "model",
    candidateKind: "paper_overview_unit_v1", schemaVersion: 1, currentRevision: 1,
    reviewState: "unreviewed", validationStatus: "valid",
    originalPayloadHash: "a".repeat(64), createdAt: "now", updatedAt: "now",
    evidenceBundle: {
      id: "bundle", runId: "run", scopeHash: "b".repeat(64),
      retrievalConfigVersion: "selection-v1", validationStatus: "valid", createdAt: "now",
      items: [{
        id: "evidence", citationKey: "citation", ordinal: 0, anchorId: "anchor",
        paperId: "paper", paperTitle: "Paper", attachmentId: "attachment",
        contentSha256: "c".repeat(64), pageIndex: 0, quoteExact: "quote",
        quotePrefix: "", quoteSuffix: "", contextText: "quote",
        relationKind: "direct_quote", validationStatus: "valid",
        currentAnchorStatus: "exact", createdAt: "now",
      }],
    },
    revisions: [{
      id: "revision", revision: 1, payloadHash: "d".repeat(64), editorKind: "ai",
      changeReason: "generated", createdAt: "now",
      payload: {
        kind: "paper_overview_unit_v1", schemaVersion: 1, profileId: "generic_overview_v1",
        fields: [
          { field: "research_question", status: "filled", statements: [{ id: "statement", text: "AI question", epistemicRole: "source_paraphrase", evidenceItemIds: ["citation"] }] },
          { field: "approach", status: "not_found", statements: [] },
          { field: "key_result", status: "not_found", statements: [] },
          { field: "conclusion", status: "not_found", statements: [] },
          { field: "limitation", status: "not_found", statements: [] },
          { field: "open_question", status: "not_found", statements: [] },
        ],
      },
    }],
    decisions: [], attentionFeedback: [], suggestionItemDecisions: [],
    citationAssessments: citationReady ? [{
      id: "assessment", candidateId: "candidate", candidateRevision: 1,
      statementId: "statement", evidenceItemId: "evidence", assessmentRevision: 1,
      semanticRelation: "supports", rationale: "checked", idempotencyKey: "assessment-key",
      assessedAt: "now",
    }] : [], overviewFieldConfirmations: [],
  };
}

function item(): LiteratureItem {
  return {
    id: "paper", itemType: "journalArticle", title: "Paper", abstractText: "",
    containerTitle: "", volume: "", issue: "", pages: "", doi: "", url: "",
    language: "", bibtexKey: "", creators: [], tags: [], readingStatus: "unread",
    attachments: [], projectIds: [], metadataSources: [], createdAt: "now", updatedAt: "now",
  };
}

function confirmedResult(): AiOverviewConfirmationResult {
  const confirmation = {
    id: "confirmation", batchId: "batch", candidateId: "candidate",
    candidateRevision: 1, fieldKind: "research_question", fieldPayloadHash: "e".repeat(64),
    decisionKind: "ai_unchanged" as const, targetReadingUnitId: "unit",
    targetAppliedRevision: 1, active: true, createdAt: "now",
  };
  return {
    candidate: {
      ...candidate({ citationReady: true }), reviewState: "accepted_unchanged",
      overviewFieldConfirmations: [confirmation],
    },
    batch: {
      id: "batch", candidateId: "candidate", candidateRevision: 1,
      scopeKind: "field", paperId: "paper", idempotencyKey: "key",
      createdAt: "now", fieldConfirmations: [confirmation], active: true,
    },
  };
}

describe("StructuredReadingCandidatePanel", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    vi.clearAllMocks();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  function renderPanel(value: AiCandidate, gateway: AiAssistanceGateway, onKnowledgeChanged = vi.fn(async () => undefined)): Promise<void> {
    return act(async () => root.render(<StructuredReadingCandidatePanel candidates={[value]} formalUnits={[]} gateway={gateway} item={item()}
      onCandidatesChanged={() => undefined} onKnowledgeChanged={onKnowledgeChanged} onPaperReviewChanged={vi.fn(async () => undefined)} />));
  }
  it("displays AI reading immediately without creating human review records or exposing forced batch review", async () => {
    const confirmOverviewFields = vi.fn();
    await renderPanel(candidate(), { confirmOverviewFields } as unknown as AiAssistanceGateway);
    expect(container.textContent).toContain("AI question");
    expect(container.querySelector('.content-trust-indicator.untrusted')).not.toBeNull();
    expect(container.textContent).not.toContain("model");
    expect(container.textContent).not.toContain("确认本块");
    expect(container.textContent).not.toContain("需先");
    expect(confirmOverviewFields).not.toHaveBeenCalled();
    expect(container.querySelector('[aria-label="跳转到原文"]')).toBeNull();
    expect(container.querySelector(".reading-field-sources button")).not.toBeNull();
  });
  it("allows optional one-click review without citation judgments, and keeps undo", async () => {
    const confirmOverviewFields = vi.fn(async () => confirmedResult());
    const onKnowledgeChanged = vi.fn(async () => undefined);
    await renderPanel(candidate(), { confirmOverviewFields } as unknown as AiAssistanceGateway, onKnowledgeChanged);
    const action = container.querySelector<HTMLButtonElement>('[aria-label="研究问题标为已审核"]')!;
    expect(action.disabled).toBe(false);
    await act(async () => action.click());
    expect(confirmOverviewFields).toHaveBeenCalledWith(expect.objectContaining({ candidateId: "candidate", candidateRevision: 1, fieldKinds: ["research_question"], scopeKind: "field" }));
    expect(onKnowledgeChanged).toHaveBeenCalledOnce();
    expect(notifications.success).toHaveBeenCalledWith("已标为人工审核。");
  });
  it("keeps persisted review marks reversible after reopening, without a confirmation dialog", async () => {
    const original = confirmedResult();
    const undoOverviewConfirmation = vi.fn(async () => ({ candidate: { ...original.candidate, reviewState: "unreviewed" as const,
      overviewFieldConfirmations: original.candidate.overviewFieldConfirmations.map(value => ({ ...value, active: false })) }, batch: { ...original.batch, active: false } }));
    function Harness(): React.JSX.Element {
      const [values, setValues] = useState<readonly AiCandidate[]>([original.candidate]);
      return <StructuredReadingCandidatePanel candidates={values} formalUnits={[]} gateway={{ undoOverviewConfirmation } as unknown as AiAssistanceGateway} item={item()}
        onCandidatesChanged={setValues} onKnowledgeChanged={vi.fn(async () => undefined)} onPaperReviewChanged={vi.fn(async () => undefined)} />;
    }
    await act(async () => root.render(<Harness />));
    expect(container.querySelector('[aria-label="撤销人工审核标记"]')).toBeNull();
    await act(async () => container.querySelector(".reading-category-card header")!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 100, clientY: 100 })));
    const reset = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(value => value.textContent === "恢复审核状态")!;
    await act(async () => reset.click());
    expect(undoOverviewConfirmation).toHaveBeenCalledWith(expect.objectContaining({ batchId: "batch", fieldKind: "research_question" }));
    expect(container.textContent).toContain("AI question");
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });
  it("does not remove a partially reviewed reading version while formal fields remain", async () => {
    const decideCandidate = vi.fn();
    await renderPanel({ ...confirmedResult().candidate, reviewState: "partially_accepted" }, { decideCandidate } as unknown as AiAssistanceGateway);
    expect(container.querySelector('[aria-label="移除本次阅读内容"]')).toBeNull();
    expect(decideCandidate).not.toHaveBeenCalled();
  });
  it("keeps content and its original color if optional review fails", async () => {
    const confirmOverviewFields = vi.fn(async () => { throw new Error("version conflict"); });
    await renderPanel(candidate(), { confirmOverviewFields } as unknown as AiAssistanceGateway);
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="研究问题标为已审核"]')!.click());
    expect(container.textContent).toContain("AI question");
    expect(container.querySelector('.content-trust-indicator.untrusted')).not.toBeNull();
    expect(notifications.error).toHaveBeenCalled();
  });
  it("lists each source directly and navigates to its own literal quote and page", async () => {
    const base = candidate();
    const evidence = base.evidenceBundle.items[0]!;
    const payload = base.revisions[0]!.payload;
    if (payload.kind !== "paper_overview_unit_v1") throw new Error("fixture kind");
    const second = { ...evidence, id: "second", citationKey: "citation2", ordinal: 1, pageIndex: 4, quoteExact: "Before literal after" };
    const field = { ...payload.fields[0]!, statements: [{ ...payload.fields[0]!.statements[0]!, evidenceItemIds: ["citation", "citation2", "citation"], evidenceQuotes: [{ evidenceItemId: "citation2", quote: "literal" }] }] };
    const value = { ...base, evidenceBundle: { ...base.evidenceBundle, items: [evidence, second] }, revisions: [{ ...base.revisions[0]!, payload: { ...payload, fields: [field, ...payload.fields.slice(1)] } }] };
    const recordEvidenceInspection = vi.fn(async () => undefined);
    const listCandidates = vi.fn(async () => [value]);
    const inspect = vi.fn((event: Event) => { event.preventDefault(); (event as CustomEvent<AiSourceInspectionRequest>).detail.resolve(); });
    window.addEventListener("research-assistant:inspect-reading-source", inspect);
    try {
      await renderPanel(value, { recordEvidenceInspection, listCandidates } as unknown as AiAssistanceGateway);
      expect(container.querySelectorAll('.reading-field-sources button')).toHaveLength(2);
      expect(container.querySelector('[role="menu"]')).toBeNull();
      await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="研究问题原文 2 · 第 5 页"]')!.click());
      expect(inspect).toHaveBeenCalledOnce();
      expect((inspect.mock.calls[0]![0] as CustomEvent<AiSourceInspectionRequest>).detail.source).toMatchObject({ pageIndex: 4, quoteExact: "literal", quotePrefix: "Before ", quoteSuffix: " after" });
      expect(recordEvidenceInspection).toHaveBeenCalledWith("candidate", 1, "second");
      expect(fieldReadingSources(value, field)).toHaveLength(2);
      expect(latestReadingSources([value], "another-paper")).toEqual([]);
      expect(latestReadingSources([{ ...value, reviewState: "rejected" }], "paper")).toEqual([]);
      expect(readingFieldColor(field.field)).toBe(container.querySelector<HTMLElement>('[data-review-field="research_question"]')?.style.getPropertyValue('--reading-field-color'));
    } finally { window.removeEventListener("research-assistant:inspect-reading-source", inspect); }
  });
});
