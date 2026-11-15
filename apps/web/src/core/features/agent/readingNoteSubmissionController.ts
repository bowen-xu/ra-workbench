import {decodeAgentReadingNotePrepareRequest,decodeAgentReadingNoteReceipt,type AgentReadingNotePrepareRequest,type AgentReadingNoteReceipt,type AgentReadingNoteSubmissionPreview} from "@research-assistant/core/agent";
import type {AgentReadingNoteSubmissionGateway,AgentTurnGateway} from "@research-assistant/data/agent";
import {ReviewedSubmissionController,type ReviewedSubmissionSnapshot} from "./reviewedSubmissionController";
import {ReviewedSubmissionOwnerRegistry} from "./reviewedSubmissionOwner";

export type ReadingNoteSubmissionSnapshot = ReviewedSubmissionSnapshot<AgentReadingNotePrepareRequest,AgentReadingNoteReceipt,AgentReadingNoteSubmissionPreview>;
export interface ReadingNoteSubmissionScope { readonly conversationId:string; readonly readingNoteId:string; }
export class ReadingNoteSubmissionController extends ReviewedSubmissionController<AgentReadingNotePrepareRequest,AgentReadingNoteReceipt,AgentReadingNoteSubmissionPreview> {
  constructor(scope:Pick<AgentReadingNoteReceipt,"conversationId"|"readingNoteId">,gateway:AgentReadingNoteSubmissionGateway,turns:AgentTurnGateway) {
    const conversationId=scope.conversationId.toLowerCase(),readingNoteId=scope.readingNoteId.toLowerCase();
    const check=(receipt:AgentReadingNoteReceipt) => {
      if(receipt.conversationId!==conversationId || receipt.readingNoteId!==readingNoteId) throw new Error("Reading note submission scope mismatch");
      return receipt;
    };
    super({
      conversationId,
      decodeRequest(value) {
        const request=decodeAgentReadingNotePrepareRequest(value);
        check({conversationId:request.conversationId,requestId:request.requestId,readingNoteId:request.context.readingNoteId});
        return request;
      },
      decodeReceipt(value) {return check(decodeAgentReadingNoteReceipt(value));},
      receipt(request) {return {conversationId:request.conversationId,requestId:request.requestId,readingNoteId:request.context.readingNoteId};},
    },gateway,turns);
  }
}

export const readingNoteSubmissionOwners = new ReviewedSubmissionOwnerRegistry<ReadingNoteSubmissionController>(
  controller => {
    const phase = controller.getSnapshot().phase;
    return phase !== "idle" && phase !== "started";
  },
);
export function readingNoteSubmissionOwnerKey(scope: ReadingNoteSubmissionScope): string {
  return JSON.stringify(["readingNote",scope.conversationId.toLowerCase(),scope.readingNoteId.toLowerCase()]);
}
export function resetReadingNoteSubmissionOwnersForTests(): void { readingNoteSubmissionOwners.resetForTests(); }
