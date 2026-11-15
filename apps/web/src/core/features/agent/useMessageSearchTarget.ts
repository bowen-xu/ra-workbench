import { useEffect, useRef, useState, type RefObject } from "react";

export type AgentMessageSearchTarget = { readonly requestId: number } & (
  { readonly messageId: string; readonly turnId?: never } | { readonly turnId: string; readonly messageId?: never }
);
/** One explicit navigation per request; later streaming updates never pull the reader back. */
export function useMessageSearchTarget(target: AgentMessageSearchTarget | null | undefined, open: boolean,
  container: RefObject<HTMLDivElement | null>, messages: readonly { readonly id: string; readonly turnId?: string }[], ready: boolean, failed: boolean) {
  const applied = useRef<AgentMessageSearchTarget | null>(null);
  const [result, setResult] = useState<{ target: AgentMessageSearchTarget; status: "found" | "missing" | "error"; messageId?: string | undefined } | null>(null);
  useEffect(() => {
    if (!open || !target) return;
    // A turn's last item is known only after history recovery completes.
    if (target.turnId && !ready) {
      if (failed) setResult((previous) => previous?.target === target && previous.status === "error" ? previous : { target, status: "error" });
      return;
    }
    const list = container.current;
    const items = list ? [...list.querySelectorAll<HTMLElement>("[data-agent-message-id]")] : [];
    const element = target.messageId ? items.find((item) => item.dataset.agentMessageId === target.messageId)
      : items.filter((item) => item.dataset.agentMessageId?.startsWith(`${target.turnId}/`)).at(-1)
        ?? [...(list?.querySelectorAll<HTMLElement>("[data-agent-turn-id]") ?? [])].find((item) => item.dataset.agentTurnId === target.turnId);
    if (applied.current === target) {
      // Folding a process region can remove its DOM while retaining its facts.
      // Only invalidate a removed identity; never navigate again on updates.
      const retained = messages.some(message => target.messageId ? message.id === target.messageId
        : message.turnId === target.turnId || message.id.startsWith(`${target.turnId}/`));
      if (ready && !element && !retained) setResult(previous => previous?.target === target && previous.status === "missing"
        ? previous : { target, status: "missing" });
      return;
    }
    if (element && list) {
      // Scroll only the transcript, never the manuscript or the surrounding application shell.
      const rect = element.getBoundingClientRect();
      const frame = list.getBoundingClientRect();
      list.scrollTop += rect.top - frame.top - Math.max(0, (list.clientHeight - rect.height) / 2);
      // Programmatic positioning does not reliably emit a native scroll event in
      // WebKit or happy-dom. Notify the transcript controller so its bottom
      // stickiness is updated before the next animation frame.
      list.dispatchEvent(new Event("scroll"));
      list.dispatchEvent(new Event("writing-agent-preserve-position"));
      element.focus({ preventScroll: true });
      applied.current = target;
      setResult({ target, status: "found", messageId: element.dataset.agentMessageId });
    } else if (failed) {
      setResult((previous) => previous?.target === target && previous.status === "error" ? previous : { target, status: "error" });
    } else if (ready) {
      applied.current = target;
      setResult({ target, status: "missing" });
    }
  }, [target, open, container, messages, ready, failed]);
  const status = !target ? null : result?.target === target ? result.status : "loading";
  return {
    matchedId: status === "found" ? result?.messageId : undefined,
    feedback: target?.turnId ? (status === "loading" ? "正在加载来源轮次…" : status === "missing" ? "未找到分支来源轮次，请刷新历史后重试。"
      : status === "error" ? "读取来源轮次失败，请重新打开来源。" : null)
      : status === "loading" ? "正在加载命中消息…" : status === "missing" ? "未找到原搜索消息，请刷新搜索结果后重试。"
      : status === "error" ? "读取命中消息失败，请重新打开该搜索结果。" : null,
  };
}
