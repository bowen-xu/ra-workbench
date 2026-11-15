import { getAppStorage } from "../../runtime/appStorage";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { AgentConversationBinding } from "@research-assistant/core/agent";
import type { AgentDeletionGateway } from "@research-assistant/data/agent";
import { notify } from "../../services/notifications";
import { useAgentMountedGuard } from "./useAgentMountedGuard";
import { ConversationDeleteConfirmation, ConversationForceDeleteDialog } from "./ConversationDeleteConfirmation";
import { acquireConversationDeletionOwner, type ConversationDeletionController, type ConversationDeletionTarget } from "./conversationDeletionController";
import type { ConversationOperationLock } from "./conversationOperationLock";
import { deletionRecoveryAction, deletionRecoveryMessage } from "./conversationDeletionRecoveryMessage";
import { sameDeletionHost } from "./conversationDeletionPending";
import "./ConversationDeleteDialog.css";

const DELETE_CONFIRMATION_SKIP_KEY = "research-assistant.agent.delete-confirmation-skip-date";
const FORCE_DELETE_CONFIRMATION_SKIP_KEY = "research-assistant.agent.force-delete-confirmation-skip-date";

function localDateKey(date = new Date()): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

export function shouldSkipConversationDeleteConfirmation(): boolean {
  try {
    return getAppStorage().getItem(DELETE_CONFIRMATION_SKIP_KEY) === localDateKey();
  } catch {
    return false;
  }
}

/** Confirmation preferences belong to the host; previews use an isolated memory adapter. */
export interface ConversationDeleteConfirmationPreferences {
  readonly shouldSkip: () => boolean;
  readonly setSkipToday: (skip: boolean) => void;
}
function localPreferences(key: string): ConversationDeleteConfirmationPreferences {
  return {
    shouldSkip() {
      try { return getAppStorage().getItem(key) === localDateKey(); } catch { return false; }
    },
    setSkipToday(skip) {
      try {
        if (skip) getAppStorage().setItem(key, localDateKey());
        else getAppStorage().removeItem(key);
      } catch { /* restricted webviews may deny storage */ }
    },
  };
}
const localConfirmationPreferences = localPreferences(DELETE_CONFIRMATION_SKIP_KEY);
const localForceConfirmationPreferences = localPreferences(FORCE_DELETE_CONFIRMATION_SKIP_KEY);

interface Props {
  readonly conversation: ConversationDeletionTarget;
  readonly binding: AgentConversationBinding | null;
  readonly gateway: AgentDeletionGateway;
  readonly onClose: () => void;
  readonly onDeleted: (id: string) => void;
  readonly onConfirmed: (id: string) => void;
  readonly confirmationPreferences?: ConversationDeleteConfirmationPreferences;
  readonly forceConfirmationPreferences?: ConversationDeleteConfirmationPreferences;
  readonly recoveryStorage?: Storage;
  readonly recoveryLock?: ConversationOperationLock;
  readonly recoveryOnly?: boolean;
  readonly viewIdentity?: string;
}
export function ConversationDeleteDialog(props: Props): React.JSX.Element | null {
  const [frozen] = useState(() => ({conversation: structuredClone(props.conversation), binding: structuredClone(props.binding), cleanup: props.onConfirmed,
    viewIdentity: props.viewIdentity ?? null}));
  const [storage] = useState(() => {try {return props.recoveryStorage ?? getAppStorage();} catch {return null;}});
  const [controller, setController] = useState<ConversationDeletionController | null>(null);
  const sameScope = JSON.stringify([props.conversation.id, props.binding, props.viewIdentity ?? null]) === JSON.stringify([frozen.conversation.id, frozen.binding, frozen.viewIdentity]);
  useEffect(() => {
    if (!storage || !sameScope) return;
    const lease = acquireConversationDeletionOwner(frozen.conversation.id, {deletions: props.gateway, cleanup: frozen.cleanup,
      ...(props.recoveryLock ? {lock: props.recoveryLock} : {})}, storage);
    setController(lease.controller); return lease.release;
  }, [storage, props.gateway, props.recoveryLock, frozen, sameScope]);
  if (!sameScope) return null;
  if (!storage) return <ConversationDeleteConfirmation pending={false} blocked retry autoConfirm={false}
    errorMessage="本机删除恢复记录不可用，尚未发送删除。（DELETION_RECOVERY_UNAVAILABLE）" onClose={props.onClose}
    onConfirm={() => notify.error("本机删除恢复记录不可用。（DELETION_RECOVERY_UNAVAILABLE）")} />;
  if (!controller) return null;
  return <OwnedDeleteDialog {...props} conversation={frozen.conversation} binding={frozen.binding} controller={controller}
    currentScope={JSON.stringify([props.conversation.id, props.binding, props.viewIdentity ?? null])} />;
}
function OwnedDeleteDialog({conversation, binding, controller, currentScope, viewIdentity, onClose, onDeleted, recoveryOnly = false,
  confirmationPreferences = localConfirmationPreferences, forceConfirmationPreferences = localForceConfirmationPreferences}: Props & {
    readonly controller: ConversationDeletionController; readonly currentScope: string;
  }): React.JSX.Element {
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  const [restored] = useState(() => recoveryOnly || state.record !== null);
  const originalScope = JSON.stringify([conversation.id, binding, viewIdentity ?? null]), scope = useRef(currentScope); scope.current = currentScope;
  const running = useRef(false), mounted = useAgentMountedGuard();
  const record = state.record, force = record?.intent.force === true || record?.stage === "retained";
  const scopeConflict = record !== null && !sameDeletionHost(record, binding, Boolean(conversation.internal));
  const pending = ["locking", "persisting", "checking", "deleting", "finishing"].includes(state.phase);
  const retry = ["unknown", "retryable", "rejected", "committed"].includes(state.phase) || state.localIssue !== null || scopeConflict;
  const blocked = scopeConflict || state.phase === "rejected" && record?.reason !== "busy" && !state.localIssue;
  async function close(): Promise<void> {
    if (running.current || pending) return;
    if (!scopeConflict && record && !["unknown", "committed"].includes(record.stage)) {
      await controller.cancelKnown();
      if (controller.getSnapshot().localIssue) notify.error(deletionRecoveryMessage(controller.getSnapshot()));
    }
    if (mounted.current && scope.current === originalScope) onClose();
  }
  async function perform(): Promise<void> {
    if (running.current || blocked || scope.current !== originalScope) return;
    running.current = true;
    try {
      let done = false;
      const before = controller.getSnapshot();
      if (!before.record) {
        if (recoveryOnly) { if (mounted.current && scope.current === originalScope) onClose(); return; }
        done = await controller.submit(conversation, binding);
      } else if (before.localIssue) done = await controller.retryLocal();
      else if (before.phase === "retained") done = await controller.confirmForce();
      else if (before.phase === "retryable" || before.phase === "rejected" && before.record.reason === "busy") {
        if (before.status === null) await controller.check();
        done = await controller.retryOriginal();
      } else if (before.phase === "committed") done = await controller.finish();
      else if (before.phase === "ready") done = true;
      else await controller.check();
      if (!done && !restored && mounted.current && scope.current === originalScope
        && controller.getSnapshot().phase === "retained" && !controller.getSnapshot().localIssue && forceConfirmationPreferences.shouldSkip()) {
        // The existing same-day force authorization continues this user action.
        done = await controller.confirmForce();
      }
      const after = controller.getSnapshot();
      if (done) {
        notify.success(`已永久删除“${after.record?.intent.expectedTitle ?? conversation.title}”对话`);
        if (mounted.current && scope.current === originalScope) onDeleted(conversation.id);
      } else if (after.localIssue || after.cause) {
        if (!(after.phase === "retained" && mounted.current)) notify.error(deletionRecoveryMessage(after));
      }
    } finally {running.current = false;}
  }
  const actionLabel = deletionRecoveryAction(state);
  const controls = {pending, blocked, retry, autoConfirm: !restored, onClose: () => void close(), onConfirm: () => void perform(),
    feedbackTone: !state.localIssue && (state.phase === "committed" || state.phase === "retryable" && record?.stage === "pending") ? "status" as const : "error" as const,
    ...(actionLabel ? {actionLabel} : {}), ...(retry ? {errorMessage: scopeConflict
      ? `此删除请求属于其他宿主或对话分类，请返回原入口核对。（DELETION_RECOVERY_SCOPE_CONFLICT；operation=${record!.intent.requestId}）` : deletionRecoveryMessage(state)} : {})};
  return force ? <ConversationForceDeleteDialog {...controls} preferences={forceConfirmationPreferences} />
    : <ConversationDeleteConfirmation {...controls} preferences={confirmationPreferences} />;
}
