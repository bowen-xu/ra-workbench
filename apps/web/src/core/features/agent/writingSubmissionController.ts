import type {AgentSubmissionGateway, AgentTurnGateway, AgentInputQueuePreviewGateway} from "@research-assistant/data/agent";
import type {WritingQueueCancellationRecovery} from "./writingQueueCancellation";
import type {WritingSubmissionController as WritingSubmissionControllerPort} from "@research-assistant/workbench-sdk/agent-writing";
import {WritingSubmissionController} from "./writingSubmissionCoordinator";

export {WritingSubmissionController};
export type {WritingSubmissionSnapshot, WritingSubmissionAdmission} from "./writingSubmissionState";

type WritingSubmissionOwner = {
  readonly controller: WritingSubmissionController;
  views: number;
  unsubscribe: () => void;
};

const writingSubmissionOwners = new Map<string, WritingSubmissionOwner>();

/**
 * Submission ownership is scoped to a manuscript conversation rather than a
 * React panel instance.  This lets a prepare/confirm pipeline survive a panel
 * unmount and lets the next panel render the exact same durable state.
 */
export function acquireWritingSubmissionController(
  key: string,
  gateway: AgentSubmissionGateway,
  turns: AgentTurnGateway,
  queueGateway?: AgentInputQueuePreviewGateway,
  recovery?: WritingQueueCancellationRecovery,
  isAdmissionDisabled?: () => boolean,
): WritingSubmissionControllerPort {
  const existing = writingSubmissionOwners.get(key);
  if (existing) {
    existing.views += 1;
    return existing.controller;
  }
  const controller = new WritingSubmissionController(gateway, turns, queueGateway, recovery, isAdmissionDisabled);
  let owner!: WritingSubmissionOwner;
  const unsubscribe = controller.subscribe(() => {
    if (owner.views === 0 && !submissionNeedsOwner(controller) && writingSubmissionOwners.get(key) === owner) {
      writingSubmissionOwners.delete(key);
      owner.unsubscribe();
      controller.dispose();
    }
  });
  owner = { controller, views: 1, unsubscribe };
  writingSubmissionOwners.set(key, owner);
  return controller;
}

function submissionNeedsOwner(controller: WritingSubmissionControllerPort): boolean {
  const snapshot = controller.getSnapshot();
  // After admission, durable projection/history is authoritative. Keeping the
  // transient controller would make old stopped views shadow a reopened panel;
  // pre-admission phases still need the owner so a switched view can recover.
  return controller.isCancelPending() || !!snapshot.sendStopIntentUnavailable || !!snapshot.heldQueueInputId || !!snapshot.queueStopHoldUnavailable || snapshot.stopPending || controller.hasWritingPreparation() || controller.hasQueuedPreparation() || (snapshot.phase !== "idle" && snapshot.phase !== "started");
}

export function hasPendingWritingSubmissions(): boolean {
  for (const { controller } of writingSubmissionOwners.values()) {
    if (submissionNeedsOwner(controller)) return true;
  }
  return false;
}

/** Release one panel view while retaining unresolved work for a later reattach. */
export function releaseWritingSubmissionController(key: string, controller: WritingSubmissionControllerPort): void {
  const owner = writingSubmissionOwners.get(key);
  if (owner?.controller !== controller || owner.views === 0) return;
  owner.views -= 1;
  if (owner.views === 0 && !submissionNeedsOwner(controller)) {
    writingSubmissionOwners.delete(key);
    owner.unsubscribe();
    controller.dispose();
  }
}

/** Test-only cleanup; production ownership intentionally lasts for the app session. */
export function forgetWritingSubmissionController(key: string, controller: WritingSubmissionControllerPort): void {
  const owner = writingSubmissionOwners.get(key);
  if (owner?.controller !== controller) return;
  writingSubmissionOwners.delete(key);
  owner.unsubscribe();
  controller.dispose();
}

/** Test-only registry reset; production callers should release a view instead. */
export function resetWritingSubmissionControllersForTests(): void {
  for (const owner of writingSubmissionOwners.values()) {
    owner.unsubscribe();
    owner.controller.dispose();
  }
  writingSubmissionOwners.clear();
}
