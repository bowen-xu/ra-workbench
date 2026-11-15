/** Shared workbench capability; implementation and lifecycle remain owned by Core. */
export { clearRecoveryDraft, createRecoveryDraft, loadRecoveryDraft, saveRecoveryDraft } from "../services/recoveryDraftStore";
export type { RecoveryDraftIdentity } from "../services/recoveryDraftStore";
export { EDITOR_AUTOSAVE_IDLE_MS } from "../services/editorAutosave";
