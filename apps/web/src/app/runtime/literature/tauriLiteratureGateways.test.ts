import {beforeEach, expect, it, vi} from "vitest";
import {tauriLiteratureGateway} from "./tauriLiteratureGateway";
import {tauriKnowledgeGateway, tauriEvidenceGateway, tauriEvidenceMatrixGateway, tauriDerivedViewGateway, tauriReferenceRecordGateway} from "./tauriResearchGateways";
import {tauriCollectionGateway, tauriNalGateway, tauriNalQueryGateway} from "./tauriCollectionNalGateways";
import {emptyCollectionState} from "@research-assistant/core/collections";
import {emptyNalState} from "@research-assistant/core/nal";

const platform = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: platform.invoke}));
beforeEach(() => platform.invoke.mockReset());

it("keeps all attachment byte response forms and original error normalization", async () => {
  const bytes = new Uint8Array([0, 255, 42]);
  for (const value of [bytes, bytes.buffer, [...bytes]]) {
    platform.invoke.mockResolvedValueOnce(value);
    await expect(tauriLiteratureGateway.readAttachment("attachment")).resolves.toEqual(bytes);
    expect(platform.invoke).toHaveBeenLastCalledWith("read_attachment", {attachmentId: "attachment"});
  }
  const original = new Error("original failure"); platform.invoke.mockRejectedValueOnce(original);
  await expect(tauriLiteratureGateway.attachPdf("paper", "/source")).rejects.toBe(original);
  platform.invoke.mockRejectedValueOnce({message: "safe transport failure"});
  await expect(tauriLiteratureGateway.attachPdf("paper", "/source")).rejects.toThrow("safe transport failure");
});

it("keeps V2 reader state and the compatible legacy fallback", async () => {
  const state = {schemaVersion: 2, pageIndex: 7, scale: {kind: "page-width"}, top: 5, left: 3, scrollMode: "vertical", spreadMode: "none"};
  platform.invoke.mockResolvedValueOnce({pageIndex: 0, zoom: 1, viewStateJson: JSON.stringify(state)});
  await expect(tauriLiteratureGateway.getReadingState("paper", "attachment")).resolves.toEqual(state);
  platform.invoke.mockResolvedValueOnce(null);
  await expect(tauriLiteratureGateway.getReadingState("paper", "attachment")).resolves.toBeUndefined();
  platform.invoke.mockResolvedValueOnce({pageIndex: 2, zoom: 1.5, viewStateJson: "invalid legacy JSON"});
  await expect(tauriLiteratureGateway.getReadingState("paper", "attachment")).resolves.toMatchObject({pageIndex: 2, scale: {kind: "numeric", value: 1.5}});
  await tauriLiteratureGateway.saveReadingState("paper", "attachment", state as never);
  expect(platform.invoke).toHaveBeenLastCalledWith("save_reading_state", {itemId: "paper", attachmentId: "attachment", pageIndex: 7, zoom: 1, viewStateJson: JSON.stringify(state)});
});

const lists = [
  {name: "knowledge", read: () => tauriKnowledgeGateway.listReadingProfiles(), command: "list_reading_profiles", args: {}},
  {name: "evidence", read: () => tauriEvidenceGateway.listClaims("project"), command: "list_claims", args: {projectId: "project"}},
  {name: "matrix", read: () => tauriEvidenceMatrixGateway.list("project"), command: "list_evidence_matrices", args: {projectId: "project"}},
  {name: "derived", read: () => tauriDerivedViewGateway.list("project"), command: "list_derived_research_views", args: {projectId: "project"}},
  {name: "reference", read: () => tauriReferenceRecordGateway.list("paper"), command: "list_reference_records", args: {paperId: "paper"}},
];
it.each(lists)("retains $name command mapping and domain response rejection", async test => {
  platform.invoke.mockResolvedValueOnce([]).mockResolvedValueOnce({invalid: true});
  await expect(test.read()).resolves.toEqual([]);
  expect(platform.invoke).toHaveBeenLastCalledWith(test.command, test.args);
  await expect(test.read()).rejects.toThrow("不兼容");
});

it("keeps collection, NAL and query response decoding and original failure identities", async () => {
  const query = {revision: 4, definitions: [], cache: null, legacyImported: true};
  platform.invoke.mockResolvedValueOnce(emptyCollectionState()).mockResolvedValueOnce(emptyNalState()).mockResolvedValueOnce(query);
  await expect(tauriCollectionGateway.read()).resolves.toEqual(emptyCollectionState());
  await expect(tauriNalGateway.read()).resolves.toEqual(emptyNalState());
  await expect(tauriNalQueryGateway.read()).resolves.toEqual(query);
  expect(platform.invoke.mock.calls).toEqual([["read_collection_network"], ["read_nal_network"], ["read_nal_query_collections"]]);
  const failure = {code: "revision_conflict", currentRevision: 7, message: "safe conflict"};
  platform.invoke.mockRejectedValueOnce(failure);
  await expect(tauriNalGateway.mutate({} as never, 4)).rejects.toBe(failure);
  platform.invoke.mockRejectedValueOnce(failure);
  await expect(tauriCollectionGateway.mutate({} as never, 4)).rejects.toThrow("safe conflict");
});
