import { useCallback, useLayoutEffect, useRef, useState } from "react";
import type { AgentConversationBinding, AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentMetadataGateway } from "@research-assistant/data/agent";
import { agentMetadataGateway } from "../../services/agentGateway";

interface Snapshot {
  readonly key: string;
  readonly gateway: AgentMetadataGateway;
  readonly status: "loading" | "ready" | "error";
  readonly conversation: AgentConversationSummary | null;
  readonly missing: boolean;
}

/** Metadata refresh is separate from the durable-history subscription and runtime lifecycle. */
export function useConversationMetadata(id: string | null, binding: AgentConversationBinding | null, gateway: AgentMetadataGateway = agentMetadataGateway) {
  const kind = binding?.kind, resourceId = binding?.resourceId;
  const key = JSON.stringify([id, kind, resourceId]);
  const active = useRef<string | null>(null);
  const sequence = useRef(0);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const refresh = useCallback(async (): Promise<void> => {
    if (!id || !kind || !resourceId || active.current !== key) return;
    const request = ++sequence.current;
    setSnapshot({ key, gateway, status: "loading", conversation: null, missing: false });
    try {
      const conversation = await gateway.read({ id, binding: { kind, resourceId } });
      if (request === sequence.current && active.current === key) setSnapshot({ key, gateway, status: "ready", conversation, missing: false });
    } catch (error) {
      if (request === sequence.current && active.current === key) setSnapshot({ key, gateway, status: "error", conversation: null, missing: error === "notFound" });
    }
  }, [id, kind, resourceId, gateway, key]);
  useLayoutEffect(() => {
    active.current = key;
    void refresh();
    return () => { active.current = null; sequence.current += 1; };
  }, [key, refresh]);
  const current = snapshot?.key === key && snapshot.gateway === gateway ? snapshot : null;
  const status = !id ? "idle" : current?.status ?? "loading";
  return { status, conversation: current?.conversation ?? null, refresh,
    unavailableReason: status === "loading" ? "正在核对对话状态…" : status === "error"
      ? current?.missing ? "此对话已不存在或不再属于当前稿件，请刷新后重试。" : "无法核对对话状态，历史和草稿已保留，请重试。"
      : undefined };
}
