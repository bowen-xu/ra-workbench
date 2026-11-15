import type { HbsPreservation, HbsPreservationStore } from "@research-assistant/core/hbs";
import { decodeHbsPreservation } from "@research-assistant/data/hbs";
import { flushStorage } from "../../runtime/storagePersistence";
import { clearRecoveryDraft, createRecoveryDraft, loadRecoveryDraft, saveRecoveryDraft, type RecoveryDraftIdentity } from "../../services/recoveryDraftStore";

export class BrowserHbsPreservationStore implements HbsPreservationStore {
  readonly durable = true;
  constructor(private readonly options: { readonly identity: RecoveryDraftIdentity; readonly unitId: string; readonly storage: Storage }) {}
  load(): HbsPreservation | undefined {
    const result = loadRecoveryDraft(this.options.identity, this.options.storage);
    return result && decodeHbsPreservation(result.payload, this.options.unitId);
  }
  async save(value: HbsPreservation): Promise<void> {
    const decoded = decodeHbsPreservation(value, this.options.unitId);
    saveRecoveryDraft(createRecoveryDraft(this.options.identity, decoded, { revision: value.observed.version }), this.options.storage);
    await flushStorage(this.options.storage);
  }
  clear(): void { clearRecoveryDraft(this.options.identity, this.options.storage); }
}
