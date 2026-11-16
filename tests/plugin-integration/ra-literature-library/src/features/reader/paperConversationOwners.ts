import type { AgentUserInput } from "@research-assistant/core/agent";
import {agentPaperSubmissionGateway, agentPaperPrepareProvenGateway, agentPaperPrepareStatusGateway, agentPaperStartStatusGateway} from "../../../../../../apps/web/src/core/public/agent";
import {getAppStorage} from "@research-assistant/workbench-sdk/storage";
import type { ConversationDraftStore } from "../../../../../../apps/web/src/core/public/agentSubmission/conversationDraftStore";
import {PaperPreparation} from "./paperPreparation";
import type { PendingConceptRetrieval } from "@research-assistant/workbench-sdk/concept-retrieval";

export type PendingPaperSubmission = {
  readonly preview: Awaited<ReturnType<typeof agentPaperSubmissionGateway.prepare>>;
  readonly text: string;
  readonly sourceCount: number;
  readonly imageCount: number;
  readonly retrievalTraceId?: string;
  readonly queuedInputId?: string;
  readonly manualConfirmation?: boolean;
};

export type PendingRetrievalSend = PendingConceptRetrieval & { readonly requestId: string;readonly userInput:readonly AgentUserInput[];
  readonly submissionOwner?: {readonly conversationId: string; readonly requestId: string} };

export type PaperConversationOwner = {
  readonly key: string;
  pendingSubmission: PendingPaperSubmission | undefined;
  pendingRetrieval: PendingRetrievalSend | undefined;
  working: number;
  views: number;
  preparation: PaperPreparation | null;
  readonly listeners: Set<() => void>;
};

// The Reader workspace keeps this panel mounted while hidden. This registry
// also covers the shorter switch-away period so an in-flight preview can
// publish back to the conversation that owns it rather than the visible one.
const paperConversationOwners = new Map<string, PaperConversationOwner>();
export function paperConversationOwnerKey(paperId: string, conversationId: string | undefined): string {
  return JSON.stringify(["paper", paperId.toLowerCase(), conversationId?.toLowerCase() ?? null]);
}
export function publishPaperOwner(owner: PaperConversationOwner): void {
  for (const listener of owner.listeners) listener();
}
export function paperConversationOwner(paperId: string, conversationId: string | undefined): PaperConversationOwner {
  const key = paperConversationOwnerKey(paperId, conversationId);
  const existing = paperConversationOwners.get(key);
  if (existing) return existing;
  const owner: PaperConversationOwner = {
    key,
    pendingSubmission: undefined,
    pendingRetrieval: undefined,
    working: 0,
    views: 0,
    preparation: null,
    listeners: new Set<() => void>(),
  };
  paperConversationOwners.set(key, owner);
  return owner;
}
function collectPaperOwner(key: string, owner: PaperConversationOwner): void {
  if (owner.views !== 0 || owner.working !== 0 || owner.pendingSubmission !== undefined
    || owner.pendingRetrieval !== undefined || owner.preparation?.unresolved() || paperConversationOwners.get(key) !== owner) return;
  owner.preparation?.dispose();
  paperConversationOwners.delete(key);
}
export function ensurePaperPreparation(owner: PaperConversationOwner, paperId: string, conversationId: string, drafts: ConversationDraftStore): PaperPreparation {
  if (!owner.preparation) {
    owner.preparation = new PaperPreparation(agentPaperSubmissionGateway, agentPaperPrepareStatusGateway, getAppStorage(), () => publishPaperOwner(owner), {paperId, conversationId}, agentPaperPrepareProvenGateway);
    owner.preparation.configureStart(agentPaperSubmissionGateway, agentPaperStartStatusGateway);
  }
  owner.preparation.configureDraft(drafts);
  return owner.preparation;
}
export function acquirePaperConversationOwner(paperId: string, conversationId: string | undefined): { owner: PaperConversationOwner; release: () => void } {
  const key = paperConversationOwnerKey(paperId, conversationId);
  const owner = paperConversationOwner(paperId, conversationId);
  owner.views += 1;
  let released = false;
  return {
    owner,
    release: () => {
      if (released) return;
      released = true;
      owner.views = Math.max(0, owner.views - 1);
      collectPaperOwner(key, owner);
    },
  };
}

export function setPaperOwnerPendingSubmission(owner: PaperConversationOwner, value: PendingPaperSubmission | undefined): void {
  owner.pendingSubmission = value;
  publishPaperOwner(owner);
}
export function setPaperOwnerPendingRetrieval(owner: PaperConversationOwner, value: PendingRetrievalSend | undefined): void {
  owner.pendingRetrieval = value;
  publishPaperOwner(owner);
}
export function beginPaperOwnerWork(owner: PaperConversationOwner): void {
  owner.working += 1;
}
export function endPaperOwnerWork(key: string, owner: PaperConversationOwner): void {
  owner.working = Math.max(0, owner.working - 1);
  collectPaperOwner(key, owner);
}
export function resetPaperConversationOwnersForTests(): void {
  for (const owner of paperConversationOwners.values()) {owner.preparation?.dispose(); owner.listeners.clear();}
  paperConversationOwners.clear();
}
