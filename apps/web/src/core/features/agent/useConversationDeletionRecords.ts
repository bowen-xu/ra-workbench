import { useEffect, useState } from "react";
import { getAppStorage } from "../../runtime/appStorage";
import { discoverAllConversationDeletions, subscribeConversationDeletions, type ConversationDeletionRecord } from "./conversationDeletionPending";
import { discoverConversationDeletionBatches, type ConversationDeletionBatch } from "./conversationDeletionBatchPending";
import { retireLegacyConversationDeletionBatches } from "./retireLegacyConversationDeletionBatches";
import type { ConversationOperationLock } from "./conversationOperationLock";
import { notify } from "../../services/notifications";

/** Only reads local recovery data. Mount, focus and invalidations never run IPC. */
export function useConversationDeletionRecords(override?: Storage, lock?: ConversationOperationLock) {
  let storage: Storage | null;
  try {storage = override ?? getAppStorage();} catch {storage = null;}
  const [found, setFound] = useState<{storage: Storage | null; records: readonly ConversationDeletionRecord[]; batches: readonly ConversationDeletionBatch[]; issue: string | null; legacyReady: boolean} | null>(null);
  useEffect(() => {
    let active = true;
    let legacyReady = false, retiring = false, legacyError = false;
    const refresh = () => {
      if (!active) return;
      let issue: string | null = null;
      const report = (cause: unknown) => {issue = cause instanceof Error && ["DELETION_RECOVERY_INVALID", "DELETION_BATCH_INVALID"].includes(cause.message)
        ? cause.message : "DELETION_RECOVERY_UNAVAILABLE";};
      let records: readonly ConversationDeletionRecord[] = [];
      let batches: readonly ConversationDeletionBatch[] = [];
      try {
        if (!storage) throw new Error("DELETION_RECOVERY_UNAVAILABLE");
        records = discoverAllConversationDeletions(storage, report);
        batches = discoverConversationDeletionBatches(storage, report);
        if (batches.length && legacyReady) {legacyReady = false; queueMicrotask(() => void retire());}
      } catch (cause: unknown) {report(cause);}
      setFound({storage, records, batches, issue, legacyReady});
    };
    const retire = async () => {
      if (!active || !storage || retiring) return;
      retiring = true;
      try {await retireLegacyConversationDeletionBatches(storage, lock); legacyReady = true; if (legacyError) {notify.dismiss("legacy-conversation-clear"); legacyError = false;} refresh();}
      catch (cause: unknown) {
        if (!active) return;
        const code = cause instanceof Error && /^DELETION_BATCH_[A-Z_]+$/u.test(cause.message) ? cause.message : "DELETION_BATCH_RECOVERY_UNAVAILABLE";
        setFound(previous => ({storage, records: previous?.records ?? [], batches: previous?.batches ?? [], issue: previous?.issue ?? null, legacyReady: false}));
        legacyError = true;
        notify.error(`无法关闭旧的清空操作，请稍后重试。（${code}）`, {id: "legacy-conversation-clear", actions: [
          {id: "retry", label: "重试", onClick: () => retire()},
        ]});
      } finally {retiring = false;}
    };
    const stop = storage ? subscribeConversationDeletions(storage, refresh) : () => undefined;
    const onFocus = () => {if (!legacyReady) void retire();};
    window.addEventListener("focus", onFocus);
    refresh(); void retire(); return () => {active = false; stop(); window.removeEventListener("focus", onFocus);};
  }, [storage, lock]);
  const current = found?.storage === storage ? found : null;
  return {storage, records: current?.records ?? [], batches: current?.batches ?? [], issue: current?.issue ?? null, legacyReady: current?.legacyReady ?? false};
}
