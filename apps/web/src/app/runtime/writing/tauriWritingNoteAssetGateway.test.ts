import {beforeEach, expect, it, vi} from "vitest";
import {tauriWritingNoteAssetGateway} from "./tauriWritingNoteAssetGateway";
const native = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: native.invoke}));
beforeEach(() => native.invoke.mockReset());

it("keeps the original three fixed commands, payloads and receipts", async () => {
  const imported = {assetId: "asset", displayName: "asset.png", mimeType: "image/png"};
  const bytes = [0, 255, 42], exported = {destinationPath: "/download/asset.png"};
  native.invoke.mockResolvedValueOnce(imported).mockResolvedValueOnce(bytes).mockResolvedValueOnce(exported);
  const request = {manuscriptId: "manuscript", displayName: "image", mimeType: "image/png", bytes};
  await expect(tauriWritingNoteAssetGateway.importAsset(request)).resolves.toBe(imported);
  await expect(tauriWritingNoteAssetGateway.readAsset("manuscript", "asset")).resolves.toBe(bytes);
  const destination = {manuscriptId: "manuscript", assetId: "asset", destinationPath: "/download/asset.png"};
  await expect(tauriWritingNoteAssetGateway.exportAsset(destination)).resolves.toBe(exported);
  expect(native.invoke.mock.calls).toEqual([
    ["import_writing_note_asset", request],
    ["read_writing_note_asset", {manuscriptId: "manuscript", assetId: "asset"}],
    ["export_writing_note_asset", destination],
  ]);
  expect(Object.keys(tauriWritingNoteAssetGateway).sort()).toEqual(["exportAsset", "importAsset", "readAsset"]);
});

it("preserves native failure identity without changing its diagnostic fields", async () => {
  const failure = {code: "asset_unavailable", requestId: "request-7", message: "safe original cause"};
  native.invoke.mockRejectedValueOnce(failure);
  await expect(tauriWritingNoteAssetGateway.readAsset("manuscript", "asset")).rejects.toBe(failure);
});
