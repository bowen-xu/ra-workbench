import { useEffect, useMemo, useState } from "react";
import type { StructuredResource, StructuredReviewRequest } from "@research-assistant/core/structured-resources";
import { getStructuredResourceGateway } from "../../services/structuredResourceGateway";
import { openSourceLink } from "./sourceNavigation";
import { countWritingChangedLines } from "./presentation/writingChangeStats";
import { AgentChangeContribution } from "./AgentTurnChangeSummary";
import type { StructuredChangeTarget } from "./textChangeTargets";

interface Snapshot {
  readonly before: unknown;
  readonly after: StructuredResource;
}
interface Change {
  readonly changeId: string;
  readonly target: StructuredChangeTarget & { readonly path: string };
  readonly title: string;
}
const text = (content: unknown): string => content == null ? "" : JSON.stringify(content, null, 2);
const diagnostic = (reason: unknown): string => reason && typeof reason === "object" && "code" in reason
  ? `${String(reason.code)}: ${reason instanceof Error ? reason.message : "message" in reason ? String(reason.message) : String(reason)}`
  : reason instanceof Error ? reason.message : String(reason);
/** Whole-resource Undo must not discard pending work from an earlier change. */
function canUndo(snapshot: Snapshot | undefined, current: StructuredResource | undefined): boolean {
  return !!snapshot && !!current && current.id === snapshot.after.id
    && current.revision === snapshot.after.revision && !current.draft
    && current.pendingFieldIds.length > 0 && text(current.content) === text(snapshot.after.content)
    && text(current.reviewBaseline) === text(snapshot.before);
}

/** Contributes resource entries; persistence and review stay with this adapter. */
export function StructuredChangeCard({ changes }: { readonly changes: readonly Change[] }): React.JSX.Element {
  const [gateway] = useState(getStructuredResourceGateway);
  // Receipt identity, rather than the parent's array identity, owns the frozen
  // comparison lifetime. Live refreshes never reload historical snapshots.
  const serialized = JSON.stringify(changes);
  const entries = useMemo(() => JSON.parse(serialized) as readonly Change[], [serialized]);
  const [snapshots, setSnapshots] = useState<Record<string, Snapshot>>({});
  const [current, setCurrent] = useState<Record<string, StructuredResource | undefined>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState<Record<string, StructuredReviewRequest>>({});
  const [undone, setUndone] = useState<ReadonlySet<string>>(new Set());
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    let active = true;
    setSnapshots({}); setErrors({}); setPending({}); setUndone(new Set());
    for (const entry of entries) {
      const { target, changeId } = entry;
      void Promise.all([
        gateway.getVersion(target.resourceId, target.revision),
        target.revision === 1 ? Promise.resolve(null) : gateway.getVersion(target.resourceId, target.revision - 1),
      ]).then(([after, before]) => {
        if (after.id !== target.resourceId || after.revision !== target.revision
          || (before && (before.id !== target.resourceId || before.revision !== target.revision - 1))) {
          throw new Error("修改快照身份不匹配");
        }
        if (active) setSnapshots(value => ({ ...value, [changeId]: { before: before?.content ?? null, after } }));
      }).catch(reason => { if (active) setErrors(value => ({ ...value, [changeId]: diagnostic(reason) })); });
    }
    return () => { active = false; };
  }, [gateway, entries]);
  useEffect(() => {
    let active = true;
    let unsubscribe: (() => void) | undefined;
    const load = (): void => {
      for (const { target, changeId } of entries) {
        void gateway.get(target.resourceId).then(resource => { if (active) setCurrent(value => ({ ...value, [changeId]: resource })); })
          .catch(reason => { if (active) {
            setCurrent(value => ({ ...value, [changeId]: undefined }));
            setErrors(value => ({ ...value, [changeId]: diagnostic(reason) }));
          } });
      }
    };
    if (gateway.subscribeChanges) void gateway.subscribeChanges(change => {
      if (entries.some(entry => entry.target.resourceId === change.resourceId)) setRefresh(value => value + 1);
    }).then(stop => { if (active) { unsubscribe = stop; load(); } else stop(); })
      .catch(reason => { if (active) setErrors(value => ({ ...value, subscription: diagnostic(reason) })); });
    load();
    return () => { active = false; unsubscribe?.(); };
  }, [gateway, entries, refresh]);

  const remaining = entries.filter(entry => !undone.has(entry.changeId));
  const recovering = Object.keys(pending).length > 0;
  const enabled = recovering || (remaining.length > 0 && remaining.every(entry => canUndo(snapshots[entry.changeId], current[entry.changeId])));
  const undo = async (): Promise<boolean> => {
    if (busy || !enabled) return false;
    setBusy(true); setErrors({});
    let activeId = "review";
    try {
      const work = recovering ? entries.filter(entry => pending[entry.changeId]) : remaining;
      const requests = new Map<string, StructuredReviewRequest>();
      // Check every remaining target before starting new writes. Native expected
      // revisions still protect each commit against changes after this check.
      for (const { target, changeId } of work) {
        activeId = changeId;
        const original = pending[changeId];
        if (original) { requests.set(changeId, original); continue; }
        const latest = await gateway.get(target.resourceId);
        if (!canUndo(snapshots[changeId], latest)) throw new Error("资源已变化，请在原位置核对并逐处撤销。");
        requests.set(changeId, { resourceId: target.resourceId, expectedRevision: latest.revision,
          requestId: crypto.randomUUID(), action: "undo", scope: "all" });
      }
      for (const { changeId } of work) {
        activeId = changeId;
        const request = requests.get(changeId)!;
        setPending(value => ({ ...value, [changeId]: request }));
        const resource = await gateway.review(request);
        setCurrent(value => ({ ...value, [changeId]: resource }));
        setUndone(value => new Set([...value, changeId]));
        setPending(value => { const next = { ...value }; delete next[changeId]; return next; });
      }
      return true;
    } catch (reason) {
      if (reason && typeof reason === "object" && "code" in reason && ["revision_conflict", "invalid_input", "not_found"].includes(String(reason.code))) {
        setPending(value => { const next = { ...value }; delete next[activeId]; return next; });
      }
      setErrors(value => ({ ...value, [activeId]: diagnostic(reason) }));
      setRefresh(value => value + 1);
      return false;
    } finally { setBusy(false); }
  };
  const items = entries.map(({ changeId, title, target }) => {
    const snapshot = snapshots[changeId];
    return { id: changeId, identity:snapshot?.after.sourceUri ?? `structured-resource:${changeId}`, label: title, onOpen: () => openSourceLink(snapshot?.after.sourceUri ?? target.path),
      ...(snapshot ? { stats: countWritingChangedLines(text(snapshot.before), text(snapshot.after.content)) } : {}) };
  });
  const stats = items.every(item => item.stats) ? items.reduce((total, item) => ({
    additions: total.additions + item.stats!.additions, deletions: total.deletions + item.stats!.deletions,
  }), { additions: 0, deletions: 0 }) : undefined;
  return <><AgentChangeContribution ariaLabel="资源变更审阅"
    {...(stats ? { stats } : {})} items={items} undo={{busy,disabled:busy||!enabled,run:undo,label:recovering?"恢复 Undo":"Undo",
      hint:enabled?"撤销本轮资源修改":"资源已确认、存在其他修改或快照尚未就绪，请在原位置审阅。"}}>
    {undone.size > 0 && undone.size < entries.length && <p role="status">已撤销 {undone.size}/{entries.length} 项修改，其余修改保留。</p>}
    {Object.entries(errors).map(([id, error]) => <p role="alert" key={id}>{entries.find(entry => entry.changeId === id)?.title ?? "资源修改"}：{error}</p>)}
  </AgentChangeContribution>
  </>;
}
