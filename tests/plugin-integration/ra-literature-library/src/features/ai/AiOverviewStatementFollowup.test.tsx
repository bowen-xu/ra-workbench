/** @vitest-environment happy-dom */
import { notify } from "@research-assistant/workbench-sdk/feedback";
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn(), warning: vi.fn() } }));

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { AiAtomicStatementV1, AiEvidenceItem } from "@research-assistant/core/ai";
import type { NotesGateway } from "@research-assistant/data/notes";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

import type { EvidenceGateway } from "../../../../../../../plugins/ra-literature-library/src/services/evidenceGateway";
import type { ResearchKnowledgeGateway } from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import { AiOverviewStatementFollowup } from "./AiOverviewStatementFollowup";

const statement: AiAtomicStatementV1 = {
  id: "statement-1",
  text: "AI suggests that the method improves accuracy.",
  epistemicRole: "source_paraphrase",
  evidenceItemIds: ["evidence-1"],
};

function evidence(currentAnchorStatus: AiEvidenceItem["currentAnchorStatus"] = "exact"): AiEvidenceItem {
  return {
    id: "evidence-1", citationKey: "ev-1", ordinal: 0, anchorId: "anchor-1",
    paperId: "paper-1", paperTitle: "Paper", attachmentId: "attachment-1",
    contentSha256: "a".repeat(64), pageIndex: 2, pageLabel: "3",
    quoteExact: "The measured accuracy increased.", quotePrefix: "before", quoteSuffix: "after",
    contextText: "The measured accuracy increased in the benchmark.", relationKind: "direct_quote",
    validationStatus: "valid", currentAnchorStatus, createdAt: "now",
  };
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")].find((item) => item.textContent === label);
  if (match === undefined) throw new Error(`missing button ${label}`);
  return match;
}

describe("overview statement human follow-up", () => {
  let container: HTMLDivElement;
  let root: Root;
  let createClaimWithEvidence: ReturnType<typeof vi.fn>;
  let createReadingNote: ReturnType<typeof vi.fn>;
  let captureConceptUsage: ReturnType<typeof vi.fn>;
  let onKnowledgeChanged: Mock<() => void>;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    createClaimWithEvidence = vi.fn(async () => ({}));
    createReadingNote = vi.fn(async () => ({}));
    captureConceptUsage = vi.fn(async () => ({}));
    onKnowledgeChanged = vi.fn();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function render(source: AiEvidenceItem = evidence()): Promise<void> {
    await act(async () => root.render(<AiOverviewStatementFollowup
      evidenceGateway={{ createClaimWithEvidence } as unknown as EvidenceGateway}
      evidenceItems={[source]}
      knowledgeGateway={{ captureConceptUsage, searchConcepts: vi.fn(async () => []) } as unknown as ResearchKnowledgeGateway}
      notesGateway={{ createReadingNote } as unknown as NotesGateway}
      onKnowledgeChanged={onKnowledgeChanged}
      projectId="project-1"
      statement={statement}
    />));
  }

  it("starts a blank human claim and persists only the user's text with the selected anchor", async () => {
    await render();
    await act(async () => button(container, "写观点").click());
    const textarea = container.querySelector("textarea");
    expect(textarea?.value).toBe("");
    expect(container.textContent).toContain(statement.text);

    await act(async () => {
      if (textarea === null) return;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set
        ?.call(textarea, "My checked interpretation.");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => button(container, "保存人工观点").click());

    expect(createClaimWithEvidence).toHaveBeenCalledWith(expect.objectContaining({
      projectId: "project-1", statement: "My checked interpretation.",
      changeReason: "manual_from_overview_source",
    }), expect.objectContaining({ anchorId: "anchor-1", stance: "context" }));
    expect(createClaimWithEvidence).not.toHaveBeenCalledWith(expect.objectContaining({
      statement: statement.text,
    }), expect.anything());
    expect(onKnowledgeChanged).toHaveBeenCalledOnce();
  });

  it("restores focus to the action that opened a cancelled editor", async () => {
    await render();
    const trigger = button(container, "写观点");
    trigger.focus();
    await act(async () => trigger.click());
    expect(document.activeElement).toBe(container.querySelector("textarea"));
    await act(async () => button(container, "取消").click());
    expect(document.activeElement).toBe(trigger);
  });

  it("keeps the human draft editable and announces a recoverable save failure", async () => {
    createClaimWithEvidence.mockRejectedValueOnce(new Error("temporary database failure"));
    await render();
    await act(async () => button(container, "写观点").click());
    const textarea = container.querySelector("textarea");
    await act(async () => {
      if (textarea === null) return;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set
        ?.call(textarea, "Draft that must survive.");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => button(container, "保存人工观点").click());

    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("草稿仍保留"));
    expect(container.querySelector("textarea")?.value).toBe("Draft that must survive.");
    expect(button(container, "保存人工观点").disabled).toBe(false);
  });

  it("creates a manual reading note by reusing the frozen evidence anchor", async () => {
    await render();
    await act(async () => button(container, "写阅读笔记").click());
    const textarea = container.querySelector("textarea");
    await act(async () => {
      if (textarea === null) return;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set
        ?.call(textarea, "A note I wrote after checking the source.");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => button(container, "保存人工笔记").click());

    expect(createReadingNote).toHaveBeenCalledWith(expect.objectContaining({
      sourceAnchorId: "anchor-1", body: "A note I wrote after checking the source.",
      quoteExact: "The measured accuracy increased.",
    }));
    expect(createReadingNote).not.toHaveBeenCalledWith(expect.objectContaining({ body: statement.text }));
  });

  it("opens a blank, disambiguated manual concept form against the selected source", async () => {
    await render();
    await act(async () => button(container, "添加概念").click());
    const term = [...container.querySelectorAll("input")]
      .find((input) => input.closest("label")?.textContent?.startsWith("语词"));
    const explanation = [...container.querySelectorAll("textarea")]
      .find((textarea) => textarea.closest("label")?.textContent?.startsWith("详细解释"));
    expect(term?.value).toBe("");
    await act(async () => {
      if (term === undefined || explanation === undefined) return;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set
        ?.call(term, "Calibration");
      term.dispatchEvent(new Event("input", { bubbles: true }));
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set
        ?.call(explanation, "A domain-specific interpretation written by the user.");
      explanation.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => button(container, "保存到知识库").click());

    expect(captureConceptUsage).toHaveBeenCalledWith(expect.objectContaining({
      anchorId: "anchor-1", termText: "Calibration",
      resolution: expect.objectContaining({ kind: "create_new" }),
    }), expect.objectContaining({ paperId: "paper-1", quoteExact: "The measured accuracy increased." }));
  });

  it("blocks all follow-up creation when the citation locator is stale", async () => {
    await render(evidence("stale"));
    expect(button(container, "写观点").disabled).toBe(true);
    expect(button(container, "写阅读笔记").disabled).toBe(true);
    expect(button(container, "添加概念").disabled).toBe(true);
    expect(container.textContent).toContain("不能用快照伪造来源");
  });
});
