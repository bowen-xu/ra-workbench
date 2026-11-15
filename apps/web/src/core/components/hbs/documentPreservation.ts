import type { DocumentCheckpoint, DocumentPreservation, HbsDocumentAdapter } from "@research-assistant/core/hbs";
import { decodeDocumentCheckpoint } from "@research-assistant/data/hbs";
import { getAppStorage } from "../../runtime/appStorage";
import { flushStorage } from "../../runtime/storagePersistence";
import { createRecoveryDraft, loadRecoveryDraft, saveRecoveryDraft, type RecoveryDraftIdentity } from "../../services/recoveryDraftStore";

/** Recovery-store adapter for hosts that inject a durable Storage port.
 * The HBS adapter preview uses its own direct IndexedDB preservation port. */
export function documentPreservation<D>(identity: RecoveryDraftIdentity, adapter: HbsDocumentAdapter<D>, storage?: Storage): DocumentPreservation<D> {
  const validate = (input: unknown): DocumentCheckpoint<D> => decodeDocumentCheckpoint(input, adapter);
  return {
    load: () => { const stored = loadRecoveryDraft(identity, storage); return stored ? validate(stored.payload) : undefined; },
    save: async value => {
      const target = storage ?? getAppStorage();
      saveRecoveryDraft(createRecoveryDraft(identity, validate(value), { revision: value.observed.version }), target);
      await flushStorage(target);
    },
  };
}
