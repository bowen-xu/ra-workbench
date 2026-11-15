import {expect, it, vi} from "vitest";
import fixture from "../../../../../../packages/writing/fixtures/document-checkpoint-v1.json";
import receiptFixture from "../../../../../../packages/writing/fixtures/document-checkpoint-editor-receipt-v1.json";
import {tauriWritingCheckpointGateway} from "./tauriWritingCheckpointGateway";
import {invoke} from "../../../core/runtime/platformCore";
vi.mock("../../../core/runtime/platformCore", () => ({invoke: vi.fn()}));

it("uses registered native commands and validates their IPC responses", async () => {
  const checkpoint = { ...fixture, schemaVersion: 1 as const };
  const row = { revision: 1, checkpoint };
  vi.mocked(invoke).mockResolvedValueOnce(row).mockResolvedValueOnce([row]).mockResolvedValueOnce([{ revision: 0, checkpoint }]);
  expect(await tauriWritingCheckpointGateway.save({ expectedRevision: null, checkpoint })).toEqual(row);
  expect(invoke).toHaveBeenCalledWith("save_writing_document_checkpoint", { write: { expectedRevision: null, checkpoint } });
  expect(await tauriWritingCheckpointGateway.list(checkpoint.manuscriptId)).toEqual([row]);
  await expect(tauriWritingCheckpointGateway.list(checkpoint.manuscriptId)).rejects.toThrow();
});

it("rejects valid-looking native acknowledgements for another file or manuscript", async () => {
  const checkpoint = { ...fixture, schemaVersion: 1 as const };
  vi.mocked(invoke).mockResolvedValueOnce({ revision: 1, checkpoint: { ...checkpoint, relativePath: "other.tex" } })
    .mockResolvedValueOnce([{ revision: 1, checkpoint: { ...checkpoint, manuscriptId: "other" } }]);
  await expect(tauriWritingCheckpointGateway.save({ expectedRevision: null, checkpoint })).rejects.toThrow("不匹配");
  await expect(tauriWritingCheckpointGateway.list(checkpoint.manuscriptId)).rejects.toThrow("不匹配");
});

it("does not move browser SHA validation into the existing native transport", async () => {
  const checkpoint = {...receiptFixture, schemaVersion: 1 as const,
    editorReceipt: {...receiptFixture.editorReceipt, text: receiptFixture.content}};
  const row = {revision: 1, checkpoint};
  vi.mocked(invoke).mockResolvedValueOnce(row);
  await expect(tauriWritingCheckpointGateway.save({expectedRevision: null, checkpoint})).resolves.toEqual(row);
});

it("retains the original native failure object and its identifiers", async () => {
  const failure = {code: "revision_conflict", operationId: "operation-7", message: "safe checkpoint cause"};
  vi.mocked(invoke).mockRejectedValueOnce(failure);
  await expect(tauriWritingCheckpointGateway.list(fixture.manuscriptId)).rejects.toBe(failure);
});
