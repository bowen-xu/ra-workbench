import { useEffect, useRef, useState } from "react";
import type { AgentArchiveRequest, AgentConversationBinding, AgentConversationSummary } from "@research-assistant/core/agent";
import { getAppStorage } from "../../runtime/appStorage";
import { notify, type NotificationAction } from "../../services/notifications";
import { acquireConversationArchiveOwner, archiveUnresolved, type ConversationArchiveController,
  type ConversationArchivePorts, type ConversationArchiveSnapshot } from "./conversationArchiveController";
import { discoverConversationArchives, sameArchiveScope, type ConversationArchiveContext } from "./conversationArchivePending";
import { archiveRecoveryMessage } from "./conversationArchiveMessage";

interface Entry {
  readonly controller: ConversationArchiveController;
  readonly release: () => void;
  detach: () => void;
  eligible: () => boolean;
  lastIntent: AgentArchiveRequest | null;
}
interface View {
  active: boolean;
  readonly scope: string;
  readonly entries: Map<string, Entry>;
  readonly notices: Set<string>;
  readonly dismissed: Map<string, string>;
}
function archiveStorage(): Storage | null {
  try { return getAppStorage(); } catch { return null; }
}
/** Views discover retained intents even when their rows moved out of a filter.
 * The existing owner registry retains business work across view unmounts. */
export function useConversationArchives(binding: AgentConversationBinding | null, ports: ConversationArchivePorts,
  onCurrent: (current: AgentConversationSummary) => void, onRejected?: (cause: unknown) => void,
  captureAcceptance?: () => () => boolean) {
  const storage = archiveStorage(), scope = JSON.stringify(binding);
  const view = useRef<View | null>(null);
  const callbacks = useRef({ onCurrent, onRejected, captureAcceptance }); callbacks.current = { onCurrent, onRejected, captureAcceptance };
  const applied = useRef(new Map<string, string>());
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  function current(owner: View): boolean { return owner.active && view.current === owner && owner.scope === scope; }
  function notice(owner: View, id: string): string { return `conversation-archive:${owner.scope}:${id}`; }
  function dismiss(owner: View, id: string): void {
    const key = notice(owner, id);
    if (owner.notices.delete(key)) notify.dismiss(key);
  }
  async function perform(owner: View, id: string, action: () => Promise<AgentConversationSummary | null>, requestedAcceptance?: () => boolean): Promise<void> {
    const accept = requestedAcceptance ?? callbacks.current.captureAcceptance?.() ?? (() => true);
    const entry = owner.entries.get(id);
    if (!entry || !current(owner) || !accept()) return;
    const context = entry.controller.getSnapshot().context;
    if (context && !sameArchiveScope(context, { binding, internal: context.internal })) return;
    entry.eligible = accept;
    const result = await action();
    if (!current(owner) || !accept()) return;
    const state = entry.controller.getSnapshot();
    if (result) {
      const signature = JSON.stringify([state.proof?.operationId, result]);
      if (applied.current.get(id) !== signature) { applied.current.set(id, signature); callbacks.current.onCurrent(result); }
      if (!state.localIssue) {
        notify.success(`“${result.title}”当前已${result.archived ? "归档" : "恢复"}`);
      }
    } else if (state.phase === "rejected") callbacks.current.onRejected?.(state.cause);
  }
  function show(owner: View, id: string, state: ConversationArchiveSnapshot): void {
    if (!current(owner)) return;
    setPending(previous => {
      const unresolved = archiveUnresolved(state);
      if (previous.has(id) === unresolved) return previous;
      const next = new Set(previous); if (unresolved) next.add(id); else next.delete(id); return next;
    });
    const entry = owner.entries.get(id); if (!entry) return;
    if (state.intent) entry.lastIntent = state.intent;
    if (!entry.eligible()) { dismiss(owner, id); return; }
    if (state.context && !sameArchiveScope(state.context, { binding, internal: state.context.internal })) {
      const key = notice(owner, id); owner.notices.add(key);
      notify.error("此对话的原归档操作属于其他宿主，请返回原宿主核对。（ARCHIVE_RECOVERY_SCOPE_CONFLICT）", { id: key });
      return;
    }
    if (["idle", "persisting", "saving", "committing", "checking", "reading"].includes(state.phase)) return;
    if (state.phase === "ready" && !state.localIssue) { dismiss(owner, id); return; }
    const controller = entry.controller;
    const actions: NotificationAction[] = [];
    if (state.intent && ["unknown", "notCommitted"].includes(state.phase)) actions.push({ id: "inspect", label: "核对原归档操作",
      onClick: async () => { if (current(owner)) { entry.eligible = callbacks.current.captureAcceptance?.() ?? (() => true); await controller.check(); } } });
    if (state.proof && ["committed", "readError"].includes(state.phase)) actions.push({ id: "read", label: "读取当前归档状态",
      onClick: () => perform(owner, id, () => controller.readCurrent()) });
    if (state.phase === "notCommitted" && state.status?.type === "noSqlReceipt" && !state.status.closed) actions.push({ id: "retry", label: "重试原归档操作",
      onClick: () => perform(owner, id, () => controller.retryOriginal()) });
    if (state.localIssue && state.intent) actions.push({ id: "persist", label: "重试本机恢复记录",
      onClick: () => perform(owner, id, () => controller.retryPersistence()) });
    const key = notice(owner, id);
    const message = archiveRecoveryMessage({...state, intent: state.intent ?? entry.lastIntent});
    const signature = JSON.stringify([message, actions.map(action => action.id)]);
    if (owner.dismissed.get(id) === signature) return;
    owner.dismissed.delete(id);
    owner.notices.add(key);
    notify.error(message, {id: key, ...(actions.length ? {actions, duration: Infinity, closeButton: true,
      onDismiss: () => { if (current(owner)) owner.dismissed.set(id, signature); }} : {})});
  }
  function attach(owner: View, id: string): Entry {
    if (!storage) throw new Error("ARCHIVE_RECOVERY_UNAVAILABLE");
    const known = owner.entries.get(id); if (known) return known;
    const lease = acquireConversationArchiveOwner(id, ports, storage);
    const entry: Entry = { ...lease, detach: () => {}, eligible: () => true, lastIntent: null };
    owner.entries.set(id, entry);
    entry.detach = entry.controller.subscribe(() => show(owner, id, entry.controller.getSnapshot()));
    show(owner, id, entry.controller.getSnapshot()); return entry;
  }
  useEffect(() => {
    const owner: View = { active: true, scope, entries: new Map(), notices: new Set(), dismissed: new Map() }; view.current = owner;
    setPending(new Set());
    const reportIssue = (cause: unknown) => {
      const key = notice(owner, "recovery"); owner.notices.add(key);
      notify.error(archiveRecoveryMessage({ phase: "unknown", intent: null, context: null, proof: null, status: null,
        current: null, cause: null, localIssue: cause }), { id: key });
    };
    try {
      if (!storage) throw new Error("ARCHIVE_RECOVERY_UNAVAILABLE");
      for (const record of discoverConversationArchives(storage, { binding, internal: false }, reportIssue)) attach(owner, record.intent.id);
      for (const record of discoverConversationArchives(storage, { binding, internal: true }, reportIssue)) attach(owner, record.intent.id);
    } catch (cause: unknown) { reportIssue(cause); }
    return () => {
      owner.active = false;
      for (const entry of owner.entries.values()) { entry.detach(); entry.release(); }
      for (const id of owner.notices) notify.dismiss(id);
      owner.entries.clear(); owner.notices.clear();
      if (view.current === owner) view.current = null;
    };
    // Host and data ports are fixed for this view; filter changes retain it.
  }, [scope, storage, ports.archives, ports.metadata]);
  return {
    pending,
    async change(conversation: AgentConversationSummary, accept = () => true): Promise<void> {
      const owner = view.current;
      if (!owner || !current(owner) || !accept()) return;
      if (!storage) {
        notify.error(archiveRecoveryMessage({phase: "localError", intent: null, context: null, proof: null, status: null, current: null,
          cause: null, localIssue: new Error("ARCHIVE_RECOVERY_UNAVAILABLE")})); return;
      }
      const id = conversation.id.toLowerCase(), entry = attach(owner, id);
      const context: ConversationArchiveContext = { binding: binding ? structuredClone(binding) : null, title: conversation.title, internal: Boolean(conversation.internal) };
      await perform(owner, id, () => entry.controller.submit(conversation, context), accept);
    },
  };
}
