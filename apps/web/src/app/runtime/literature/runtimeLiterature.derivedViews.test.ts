import {expect, it, vi} from "vitest";
import type {DerivedResearchViewDraft} from "@research-assistant/core/synthesis";
import {createRuntimeLiteraturePort} from "./runtimeLiterature";
const key = "d".repeat(64), caps = [{id: "literature.derived-views", version: 1}];
const draft: DerivedResearchViewDraft = {projectId: "project", viewKind: "taxonomy", title: " Compare Ω ", scopeNote: " original ", inputs: [{objectKind: "matrix_cell", objectId: "cell", objectRevision: 1, selectionReason: " chosen "}], items: [{id: "i", text: "Synthesized", epistemicRole: "synthesis", evidenceAnchorIds: ["a"], hasConflict: true}], reviewState: "accepted", changeReason: " review "};
it("routes both derived-view methods with original values and exact selected identity", async () => {
  const response = {original: true}, invoke = vi.fn(async () => response);
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke).derivedViews!;
  expect(await port.list(" project\u0000 ")).toBe(response); expect(await port.save(draft)).toBe(response);
  expect(invoke.mock.calls).toEqual([["list_runtime_derived_research_views", {packageKey: key, projectId: " project\u0000 "}], ["save_runtime_derived_research_view", {packageKey: key, draft}]]);
});
it("refuses undeclared versions and preserves safe unknown-settlement diagnostics without replay", async () => {
  const invoke = vi.fn();
  for (const provided of [[], [{id: "literature.derived-views", version: 2}]]) {
    const port = createRuntimeLiteraturePort(key, provided, () => undefined, invoke).derivedViews!;
    await expect(port.list("p")).rejects.toMatchObject({code: "literature_gateway_unavailable"});
    await expect(port.save(draft)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  }
  expect(invoke).not.toHaveBeenCalled();
  const details = {phase: "rollback", outcome: "unknown", objectId: "view-id", rollbackError: {safeCause: "sqlite_operation_failed", sqliteExtendedCode: 23}, unknownBody: "private synthetic body"};
  const cause = {code: "storage_error", message: "派生视图结果未知，禁止自动重试。", operationId: "operation", derivedView: details};
  invoke.mockRejectedValue(cause);
  const error = await createRuntimeLiteraturePort(key, caps, () => undefined, invoke).derivedViews!.save(draft).catch(value => value);
  expect(error).toMatchObject({code: "storage_error", derivedView: details, cause});
  expect(error.message).toContain("derivedView.outcome=unknown"); expect(error.message).toContain("sqliteExtendedCode=23"); expect(error.message).not.toContain("private synthetic body"); expect(invoke).toHaveBeenCalledTimes(1);
});
