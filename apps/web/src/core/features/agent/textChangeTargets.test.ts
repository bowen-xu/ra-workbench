import { expect, it } from "vitest";
import { textPatchTargets } from "./textChangeTargets";

const target = { path: "source://resource", name: "画像", status: "applied", message: "saved" };
it("normalizes old note identities and prefers the current changeId", () => {
  expect(textPatchTargets(JSON.stringify({ targets: [{ ...target, proposalId: "legacy" }] }))[0]?.changeId).toBe("legacy");
  const [decoded] = textPatchTargets(JSON.stringify({ targets: [{ ...target, proposalId: "legacy", changeId: "current" }] }));
  expect(decoded?.changeId).toBe("current");
  expect(decoded).not.toHaveProperty("proposalId");
});
it("recovers successful historical structured changes without confusing repair drafts with changes", () => {
  const structuredResource = { resourceId: "resource", revision: 3, draftId: null, receipt: { requestId: "write-receipt" } };
  const [decoded] = textPatchTargets(JSON.stringify({ targets: [{ ...target, structuredResource }] }));
  expect(decoded?.changeId).toBe("write-receipt");
  // Transcript deduplication serializes the normalized targets a second time.
  expect(textPatchTargets(JSON.stringify({ targets: [decoded] }))).toEqual([decoded]);
  expect(textPatchTargets(JSON.stringify({ targets: [{ ...target, message: JSON.stringify(structuredResource) }] }))[0]).toMatchObject({ changeId: decoded?.changeId, structuredResource: decoded?.structuredResource });
  const failed = textPatchTargets(JSON.stringify({ targets: [{ ...target, status: "failed", structuredResource: { resourceId: "resource", revision: 0, draftId: "repair" } }] }));
  expect(failed[0]?.changeId).toBeUndefined();
});
