import {decodeAgentNotePrepareRequest,decodeAgentNoteReceipt,type AgentNotePrepareRequest,type AgentNoteReceipt,type AgentNoteSubmissionPreview} from "@research-assistant/core/agent";
import type {AgentNoteSubmissionGateway,AgentTurnGateway} from "@research-assistant/data/agent";
import {ReviewedSubmissionController,type ReviewedSubmissionSnapshot} from "./reviewedSubmissionController";
import {ReviewedSubmissionOwnerRegistry} from "./reviewedSubmissionOwner";

export type NoteSubmissionSnapshot = ReviewedSubmissionSnapshot<AgentNotePrepareRequest,AgentNoteReceipt,AgentNoteSubmissionPreview>;
export interface NoteSubmissionScope { readonly conversationId:string; readonly pageId:string; }
export class NoteSubmissionController extends ReviewedSubmissionController<AgentNotePrepareRequest,AgentNoteReceipt,AgentNoteSubmissionPreview> {
  constructor(scope:Pick<AgentNoteReceipt,"conversationId"|"pageId">,gateway:AgentNoteSubmissionGateway,turns:AgentTurnGateway) {
    const conversationId=scope.conversationId.toLowerCase(),pageId=scope.pageId.toLowerCase();
    const check=(receipt:AgentNoteReceipt) => {
      if(receipt.conversationId!==conversationId || receipt.pageId!==pageId) throw new Error("Note submission scope mismatch");
      return receipt;
    };
    super({
      conversationId,
      decodeRequest(value) {
        const request=decodeAgentNotePrepareRequest(value);
        check({conversationId:request.conversationId,requestId:request.requestId,pageId:request.context.pageId});
        return request;
      },
      decodeReceipt(value) {return check(decodeAgentNoteReceipt(value));},
      receipt(request) {return {conversationId:request.conversationId,requestId:request.requestId,pageId:request.context.pageId};},
    },gateway,turns);
  }
}

/**
 * A note panel is a view over this owner.  Keeping the owner by scope lets a
 * pending prepare survive a conversation switch and be reattached when the
 * original session is selected again.
 */
export const noteSubmissionOwners = new ReviewedSubmissionOwnerRegistry<NoteSubmissionController>(
  controller => {
    const phase = controller.getSnapshot().phase;
    return phase !== "idle" && phase !== "started";
  },
);
export function noteSubmissionOwnerKey(scope: NoteSubmissionScope): string {
  return JSON.stringify(["note",scope.conversationId.toLowerCase(),scope.pageId.toLowerCase()]);
}
export function resetNoteSubmissionOwnersForTests(): void { noteSubmissionOwners.resetForTests(); }
