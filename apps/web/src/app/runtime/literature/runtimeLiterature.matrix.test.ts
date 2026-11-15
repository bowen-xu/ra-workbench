import {expect, it, vi} from "vitest";
import type {EvidenceMatrixDraft, EvidenceMatrixCellDraft} from "@research-assistant/core/synthesis";
import {createRuntimeLiteraturePort} from "./runtimeLiterature";

const key = "c".repeat(64), caps = [{id: "literature.evidence-matrix", version: 1}];
const draft: EvidenceMatrixDraft = {projectId: "project", title: " Compare Ω ", paperIds: ["a", "b"], dimensions: [{id: "d", label: "Claim"}], changeReason: " manual "};
const cell: EvidenceMatrixCellDraft = {matrixId: "matrix", paperId: "a", dimensionId: "d", coverageState: "not_found", coverageNote: " No result ", statements: [], changeReason: " review "};
it("routes the three original evidence inventory reads through their separate capability", async () => {
  const rows: unknown[] = [], invoke = vi.fn(async () => rows);
  const port = createRuntimeLiteraturePort(key, [{id: "literature.evidence-read", version: 1}], () => undefined, invoke).evidence!;
  await expect(port.listClaims(undefined)).resolves.toBe(rows);
  await expect(port.listSources(" project\u0000 ")).resolves.toBe(rows);
  await expect(port.listClaimEvidence(" claim\u0000 ")).resolves.toBe(rows);
  expect(invoke.mock.calls).toEqual([
    ["list_runtime_evidence_claims", {packageKey: key, projectId: undefined}],
    ["list_runtime_evidence_sources", {packageKey: key, projectId: " project\u0000 "}],
    ["list_runtime_claim_evidence", {packageKey: key, claimId: " claim\u0000 "}],
  ]);
  const missing = createRuntimeLiteraturePort(key, caps, () => undefined, invoke).evidence!;
  await expect(missing.listSources("project")).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  expect(invoke).toHaveBeenCalledTimes(3);
});
it("routes all four original matrix operations with exact draft bytes and the selected identity", async () => {
  const result = {original: true}, invoke = vi.fn(async () => result);
  const matrix = createRuntimeLiteraturePort(key, caps, () => undefined, invoke).evidenceMatrix!;
  for (const value of [await matrix.list(" project\u0000 "), await matrix.save(draft), await matrix.listCells(" matrix\u0000 "), await matrix.saveCell(cell)]) expect(value).toBe(result);
  expect(invoke.mock.calls).toEqual([
    ["list_runtime_evidence_matrices", {packageKey: key, projectId: " project\u0000 "}],
    ["save_runtime_evidence_matrix", {packageKey: key, draft}],
    ["list_runtime_evidence_matrix_cells", {packageKey: key, matrixId: " matrix\u0000 "}],
    ["save_runtime_evidence_matrix_cell", {packageKey: key, draft: cell}],
  ]);
});
it("keeps capability and activation checks before matrix transport and never retries an unknown settlement", async () => {
  const invoke = vi.fn();
  for (const caps of [[], [{id: "literature.evidence-matrix", version: 2}]]) {
    const matrix = createRuntimeLiteraturePort(key, caps, () => undefined, invoke).evidenceMatrix!;
    await expect(matrix.save(draft)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
    await expect(matrix.listCells("matrix")).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  }
  expect(invoke).not.toHaveBeenCalled();
  const details = {phase: "rollback", outcome: "unknown", rollbackError: {safeCause: "sqlite_operation_failed", sqliteExtendedCode: 23}, unknownBody: "private synthetic body"};
  const cause = {code: "storage_error", message: "矩阵结果未知，禁止自动重试。", operationId: "operation", evidenceMatrix: details};
  invoke.mockRejectedValue(cause);
  const error = await createRuntimeLiteraturePort(key, caps, () => undefined, invoke).evidenceMatrix!.save(draft).catch(value => value);
  expect(error).toMatchObject({code: "storage_error", operationId: "operation", evidenceMatrix: details, cause});
  expect(error.message).toContain("evidenceMatrix.outcome=unknown");
  expect(error.message).toContain("sqliteExtendedCode=23");
  expect(error.message).not.toContain("private synthetic body");
  expect(invoke).toHaveBeenCalledTimes(1);
});
