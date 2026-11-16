// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vitest";
import { getAppStorage, selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { candidateFieldSelection, resolveReadingSelection, mergeReadingSelection, overwriteFieldSelection, readReadingPresentation, saveReadingPresentation } from "../../../../../../../plugins/ra-literature-library/src/features/ai/readingPresentation";
import { overviewCandidate } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import type { AiCandidate } from "@research-assistant/core/ai";
function edited(): AiCandidate {
  const candidate = overviewCandidate("human", "paper");
  const first = candidate.revisions[0]!;
  if (first.payload.kind !== "paper_overview_unit_v1") throw new Error("fixture");
  return { ...candidate, currentRevision: 2, revisions: [first, { ...first, id: "human-revision", revision: 2, editorKind: "human", payload: { ...first.payload, fields: first.payload.fields.map(field => ({ ...field, statements: field.statements.map(statement => ({ ...statement, text: "Human correction" })) })) } }] };
}
beforeEach(() => { const data = new Map<string, string>(); selectAppStorage({ get length() { return data.size; }, key: index => [...data.keys()][index] ?? null, getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); }, removeItem: key => { data.delete(key); }, clear: () => data.clear() }); });
describe("reading overwrite selection", () => {
  it("recovers stale display pointers but preserves an explicitly cleared view", () => {
    const latest = overviewCandidate("latest", "paper");
    expect(resolveReadingSelection({ fields: [{ candidateId: "removed", fieldId: "research_question" }], hiddenUnitIds: [] }, [latest], latest)).toEqual(candidateFieldSelection(latest));
    expect(resolveReadingSelection({ fields: [], hiddenUnitIds: [] }, [latest], latest)).toEqual([]);
  });
  it("separates unreviewed AI from human edits and protects reviewed fields", () => {
    const ai = overviewCandidate("ai", "paper"), human = edited();
    const reviewed: AiCandidate = { ...overviewCandidate("reviewed", "paper"), overviewFieldConfirmations: [{ id: "confirmation", batchId: "batch", candidateId: "reviewed", candidateRevision: 1, active: true, fieldKind: "research_question", fieldPayloadHash: "f".repeat(64), decisionKind: "ai_unchanged", targetReadingUnitId: "unit", targetAppliedRevision: 1, createdAt: "now" }] };
    const fields = [ai, human, reviewed].flatMap(candidateFieldSelection);
    expect(overwriteFieldSelection(fields, [ai, human, reviewed], "ai_unreviewed")).toEqual(candidateFieldSelection(ai));
    expect(overwriteFieldSelection(fields, [ai, human, reviewed], "human_unreviewed")).toEqual(candidateFieldSelection(human));
    expect(overwriteFieldSelection(fields, [ai, human, reviewed], "all")).toEqual(fields);
  });
  it("replaces selected field pointers while preserving the original candidate and its evidence", () => {
    const original = edited(), generated = overviewCandidate("generated", "paper");
    const before = JSON.stringify(original);
    const current = candidateFieldSelection(original);
    expect(mergeReadingSelection(current, generated, [])).toEqual(current);
    expect(mergeReadingSelection(current, generated, current)).toEqual(candidateFieldSelection(generated));
    expect(JSON.stringify(original)).toBe(before);
  });
  it("persists legacy field selection and empty state without storing research text", () => {
    saveReadingPresentation("paper", { fields: [], hiddenUnitIds: ["formal"] });
    expect(readReadingPresentation("paper")).toEqual({ fields: [], hiddenUnitIds: ["formal"] });
    expect(readReadingPresentation("other-paper")).toBeUndefined();
    const fields = candidateFieldSelection(edited());
    saveReadingPresentation("paper", { fields, hiddenUnitIds: [] });
    expect(readReadingPresentation("paper")?.fields).toEqual(fields);
  });
  it("ignores obsolete revision visibility while retaining legacy field pointers", () => {
    const fields = candidateFieldSelection(edited());
    getAppStorage().setItem("research-assistant.reading-presentation.v1:paper", JSON.stringify({
      fields, hiddenUnitIds: ["formal"], hiddenResourceVersions: [{ resourceId: "reading", revision: 2 }],
    }));
    expect(readReadingPresentation("paper")).toEqual({fields, hiddenUnitIds: ["formal"]});
  });
});
