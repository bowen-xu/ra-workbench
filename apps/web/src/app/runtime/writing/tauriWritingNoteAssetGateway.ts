import {invoke} from "../../../core/runtime/platformCore";
import type {WritingNoteAssetGateway} from "@research-assistant/data/writing";

/** App owns these three fixed commands; it does not select an asset owner or
 * repeat the writing host's validation, naming, storage or recovery rules. */
export function createTauriWritingNoteAssetGateway(
  dispatch: typeof invoke,
): WritingNoteAssetGateway {
  return {
    importAsset: request => dispatch("import_writing_note_asset", request),
    readAsset: (manuscriptId, assetId) => dispatch("read_writing_note_asset", {manuscriptId, assetId}),
    exportAsset: request => dispatch("export_writing_note_asset", request),
  };
}

export const tauriWritingNoteAssetGateway = createTauriWritingNoteAssetGateway(invoke);
