import { useEffect, useRef, useState } from "react";
import { notify } from "../../services/notifications";
import { getAgentCommandGateway, type AgentCommandGateway, type AgentCommandUsage } from "./agentCommandGateway";
import type { AgentCompactionRecord } from "./AgentCompactionActivity";

/** Owns UI feedback and cancellation; the injected adapter owns compaction. */
export function useAgentCommandExecution({ gateway: suppliedGateway, conversationId, text, change, afterMessageId, enabled = true, onFailure }: {
  readonly onFailure?: () => Promise<void>;
  readonly enabled?: boolean;
  readonly gateway?: AgentCommandGateway;
  readonly conversationId?: string;
  readonly text: string;
  readonly change: (text: string) => void;
  readonly afterMessageId?: string | undefined;
}) {
  const gateway = enabled ? suppliedGateway ?? getAgentCommandGateway() : undefined;
  const [pending, setPending] = useState(false);
  const [usage, setUsage] = useState<AgentCommandUsage>();
  const [activities, setActivities] = useState<readonly AgentCompactionRecord[]>([]);
  const flight = useRef<AbortController | null>(null);
  const generation = useRef(0);
  const completedRequests = useRef(new Set<string>());
  const latest = useRef({ text, change, onFailure }); latest.current = { text, change, onFailure };
  useEffect(() => {
    const controller = new AbortController(), version = ++generation.current;
    setUsage(undefined); setPending(false); setActivities([]);
    completedRequests.current.clear();
    if (gateway && conversationId) void gateway.readUsage({ conversationId, signal: controller.signal }).then(value => {
      if (!controller.signal.aborted && generation.current === version) setUsage(value);
    }).catch(() => {
      // Missing usage is already represented by the existing context control.
      // It does not prevent an explicit command from retrying the adapter.
      if (!controller.signal.aborted && generation.current === version) setUsage(undefined);
    });
    return () => { controller.abort(); flight.current?.abort(); flight.current = null; ++generation.current; };
  }, [gateway, conversationId]);

  async function compact(value: string, { preserveDraft = false, requestId }: { readonly preserveDraft?: boolean; readonly requestId?: string;
    readonly queued?: {readonly inputId: string; readonly explicit: boolean} } = {}): Promise<boolean> {
    if (!gateway) { notify.info("上下文压缩尚未接入，当前对话保持不变。"); return false; }
    if (!conversationId) { notify.error("请先打开一个对话，再压缩上下文。"); return false; }
    if (requestId && completedRequests.current.has(requestId)) return true;
    if (flight.current) return false;
    const controller = new AbortController(); flight.current = controller;
    const version = ++generation.current;
    const activity: AgentCompactionRecord = { id: crypto.randomUUID(), conversationId, afterMessageId, requestId, state: "pending" };
    setActivities(previous => [...previous, activity]);
    setPending(true);
    if (!preserveDraft) latest.current.change("");
    try {
      const result = await gateway.compact({ conversationId, signal: controller.signal, ...(requestId ? {requestId} : {}) });
      if (controller.signal.aborted || generation.current !== version) return false;
      setUsage(result.usage);
      if (requestId) completedRequests.current.add(requestId);
      setActivities(previous => previous.map(item => item.id === activity.id ? { ...item, state: "completed", result } : item));
      return true;
    } catch {
      if (!controller.signal.aborted && generation.current === version) {
        setActivities(previous => previous.map(item => item.id === activity.id ? { ...item, state: "failed" } : item));
        // Restore a failed command only while the editor is still unused.
        if (!preserveDraft && (latest.current.text === "" || latest.current.text === value)) latest.current.change(value);
        notify.error("上下文压缩失败，草稿和对话保留，请重试。");
        // Keep dispatch blocked until the host has retained its deferred input.
        try { await latest.current.onFailure?.(); }
        catch { notify.error("无法确认后续队列的暂停结果，请核对队列状态后再继续。"); }
      }
      return false;
    } finally {
      if (flight.current === controller) { flight.current = null; setPending(false); }
    }
  }
  return { compact, pending, usage, activities };
}
export type AgentCommandExecution = ReturnType<typeof useAgentCommandExecution>;
