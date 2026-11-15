/** Existing shared Agent admission, draft receipts and recovery for manuscript
 * hosts. These exports retain the same controllers and registries used by Core;
 * a plugin never creates a parallel conversation, queue or execution runtime. */
export { acquireWritingSubmissionController, hasPendingWritingSubmissions, releaseWritingSubmissionController } from "../features/agent/writingSubmissionController";
export type {WritingSubmissionAdmission, WritingSubmissionSnapshot} from "../features/agent/writingSubmissionController";
export type {WritingSubmissionController} from "@research-assistant/workbench-sdk/agent-writing";
export { captureWritingDraftReceipt, unownedWritingDraftReceipt, writingDraftAcknowledgments, decodeWritingDraftAcknowledgment } from "../features/agent/writingDraftAcknowledgment";
export type { WritingDraftReceipt, WritingDraftAcknowledgmentPort } from "../features/agent/writingDraftAcknowledgment";
export { writingQueueCancellationRecovery, sameWritingQueueCancellationIntent } from "../features/agent/writingQueueCancellation";
export { writingQueueCancellationMessage } from "../features/agent/writingQueueCancellationMessage";
export { writingQueuePreparationMessage } from "../features/agent/writingQueuePreparationMessage";
export { writingWorkspacePreparationMessage } from "../features/agent/writingWorkspacePreparationMessage";
export { writingQueuedStartMessage } from "../features/agent/writingQueuedStartMessage";
export type { WritingQueuedStartSnapshot } from "../features/agent/writingQueuedStart";
export { writingQueuedStartQueue } from "../features/agent/writingQueuedStartQueue";
export { writingPreparationDomainFields, writingPreparationMessage } from "../features/agent/writingPreparationMessage";
export type { WritingPreparationSnapshot } from "../features/agent/writingPreparation";
export { writingPreparationRejection } from "../features/agent/writingPreparationRejection";
export type { WritingPreparationRejection } from "../features/agent/writingPreparationRejection";
export { ReviewedSubmissionController } from "../features/agent/reviewedSubmissionController";
export type { ReviewedSubmissionSnapshot } from "../features/agent/reviewedSubmissionController";
export { ReviewedSubmissionOwnerRegistry } from "../features/agent/reviewedSubmissionOwner";
