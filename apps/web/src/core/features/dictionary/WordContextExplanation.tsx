import { useEffect, useRef, useState } from "react";
import { MessageCircle, Send, Trash2, X } from "lucide-react";
import type { WordChatMessage, WordExplanation, WordExplanationRequest, WordExplanationGateway } from "@research-assistant/core/dictionary";
import { ControlHint } from "../../components/ControlHint";
import { getWordExplanationGateway } from "../../services/wordExplanationGateway";
import { notify } from "../../services/notifications";

export function WordContextExplanation({ explanation, context, gateway, onDelete, deleteDisabled = false }: {
  readonly explanation: WordExplanation;
  readonly context: WordExplanationRequest;
  readonly gateway?: WordExplanationGateway<AbortSignal>;
  readonly onDelete?: () => Promise<void>;
  readonly deleteDisabled?: boolean;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [messages, setMessages] = useState<readonly WordChatMessage[]>([]);
  const [busy, setBusy] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const request = useRef<AbortController | undefined>(undefined);
  const input = useRef<HTMLTextAreaElement | null>(null);
  useEffect(() => {
    setOpen(false); setDraft(""); setMessages([]); setBusy(false);
    return () => request.current?.abort();
  }, [explanation]);
  useEffect(() => { if (open) input.current?.focus(); }, [open]);
  const full = messages.length >= 14;
  function close(): void { request.current?.abort(); setBusy(false); setOpen(false); }
  async function send(): Promise<void> {
    if (!draft.trim() || full || (request.current && !request.current.signal.aborted)) return;
    const controller = new AbortController(); request.current = controller; setBusy(true);
    const question = draft.trim();
    const next: readonly WordChatMessage[] = [...messages, { role: "user", text: question }];
    try {
      const result = await (gateway ?? getWordExplanationGateway()).followUp({ context,
        explanation: { translation: explanation.translation, explanation: explanation.explanation }, messages: next }, controller.signal);
      if (controller.signal.aborted) return;
      setMessages([...next, { role: "assistant", text: result.text }]); setDraft("");
    } catch (error) {
      if (!controller.signal.aborted) notify.error(error instanceof Error ? error.message : "局部追问失败，请重试。");
    } finally {
      if (!controller.signal.aborted) setBusy(false);
      if (request.current === controller) request.current = undefined;
    }
  }
  async function remove(): Promise<void> {
    if (!onDelete || deleting || deleteDisabled) return;
    setDeleting(true); request.current?.abort(); setBusy(false);
    try { await onDelete(); }
    catch (error) { notify.error(error instanceof Error ? error.message : "删除解释失败，请重试。"); }
    finally { setDeleting(false); }
  }
  return <div className="dictionary-context-explanation" aria-label="AI 语境释义">
    <div className="dictionary-context-meaning"><div className="dictionary-context-title"><p className="dictionary-context-translation">{explanation.translation}</p>
      {onDelete && <ControlHint content="删除词义解释"><button className="dictionary-context-delete" aria-label="删除词义解释" disabled={deleting || deleteDisabled} type="button" onClick={() => void remove()}><Trash2 aria-hidden="true" size={16} /></button></ControlHint>}</div>
      <ControlHint content="继续追问词义"><button aria-label="继续追问词义" aria-expanded={open} disabled={deleting} type="button" onClick={() => open ? close() : setOpen(true)}><MessageCircle aria-hidden="true" size={16} /></button></ControlHint>
    </div>
    <p className="dictionary-context-reason">{explanation.explanation}</p>
    {open && <section className="dictionary-local-chat" aria-label="词义局部追问">
      <header><span>局部追问</span><ControlHint content="收起局部追问"><button aria-label="收起局部追问" type="button" onClick={close}><X aria-hidden="true" size={16} /></button></ControlHint></header>
      <div className="dictionary-local-messages" role="log" aria-live="polite">
        {messages.map((message, index) => <p key={index} className={`dictionary-chat-${message.role}`}><span>{message.role === "user" ? "你" : "回答"}</span>{message.text}</p>)}
        {busy && <p role="status">正在回答…</p>}
      </div>
      {full ? <p>本次局部对话已到长度上限，重新生成词义后可开始新对话。</p> : <form onSubmit={event => { event.preventDefault(); void send(); }}>
        <textarea ref={input} aria-label="词义追问" placeholder="进一步问问这个词的用法…" maxLength={1000} value={draft} disabled={busy}
          onChange={event => setDraft(event.target.value)} onKeyDown={event => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void send(); }
          }} />
        <ControlHint content="发送追问"><button aria-label="发送追问" type="submit" disabled={busy || !draft.trim()}><Send aria-hidden="true" size={16} /></button></ControlHint>
      </form>}
    </section>}
  </div>;
}
