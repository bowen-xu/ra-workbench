import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CodeXml, X } from "lucide-react";
import { ControlHint } from "../../components/ControlHint";
import { useDeveloperMode } from "../settings/DeveloperMode";
import { AgentMessageCopyButton } from "./AgentMessageCopyButton";
import { useAgentInlineData } from "./useAgentInlineData";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import "./AgentMessageData.css";

export type ReadMessageData = (message: WritingAgentMessage, options?: { readonly referencesOnly: true }) => Promise<unknown>;
type DataSource = { readonly messages: readonly WritingAgentMessage[]; readonly read?: ReadMessageData | undefined; readonly data?: never }
  | { readonly data: unknown; readonly messages?: never; readonly read?: never };
type MessageDataProps = { readonly className?: string } & (DataSource & { readonly presentation?: "dialog" }
  | { readonly data: unknown; readonly presentation: "inline" });
export function AgentMessageData({ className = "", presentation = "dialog", ...source }: MessageDataProps): React.JSX.Element | null {
  const { enabled } = useDeveloperMode();
  const trigger = useRef<HTMLButtonElement>(null);
  const [selected, setSelected] = useState<DataSource | null>(null);
  const inline = useAgentInlineData(source.data, className);
  useEffect(() => { if (!enabled) setSelected(null); }, [enabled]);
  if (!enabled) return null;
  if (presentation === "inline") return <>{inline.trigger}{inline.content}</>;
  return <><ControlHint content="查看原始数据"><button className={`agent-message-data-trigger ${className}`} aria-label="查看原始数据" ref={trigger} type="button"
    onClick={() => setSelected(source)}><CodeXml aria-hidden="true" size={13} /></button></ControlHint>
    {selected && <MessageDataDialog source={selected} onClose={() => { setSelected(null); trigger.current?.focus({ preventScroll: true }); }} />}</>;
}
function MessageDataDialog({ source, onClose }: { readonly source: DataSource; readonly onClose: () => void }): React.JSX.Element {
  const dialog = useRef<HTMLDialogElement>(null);
  const [data, setData] = useState<string | null>(null), [failed, setFailed] = useState(false), [attempt, setAttempt] = useState(0);
  useEffect(() => { const element = dialog.current; element?.showModal(); return () => element?.close(); }, []);
  useEffect(() => {
    let active = true; setData(null); setFailed(false);
    const request = source.messages ? Promise.all(source.messages.map(async message => ({ message: message.rawData ?? message,
      record: source.read ? await source.read(message) : null }))).then(value => value.length === 1 ? value[0] : value) : Promise.resolve(source.data);
    void request.then(value => {
      if (active) setData(JSON.stringify(value, null, 2));
    }).catch(() => { if (active) setFailed(true); });
    return () => { active = false; };
  }, [source, attempt]);
  const close = (): void => { dialog.current?.close(); onClose(); };
  return createPortal(<dialog ref={dialog} className="agent-message-data-dialog" aria-label="消息原始数据"
    onCancel={event => { event.preventDefault(); close(); }}>
    <header><h2>消息原始数据</h2><button className="agent-message-data-close" type="button" aria-label="关闭" onClick={close}><X aria-hidden="true" size={18} /></button></header>
    <div className="agent-message-data-toolbar"><p>{source.messages ? "本地记录的消息事件与本轮冻结提交；不等同于供应商 HTTP 报文。缺失的记录不会由当前设置或源码补填。" : "本地记录的命令状态与命令网关返回值；不等同于供应商 HTTP 报文。"}</p>{data !== null && <AgentMessageCopyButton className="agent-message-data-copy" value={data} />}</div>
    {data !== null ? <pre tabIndex={0}>{data}</pre>
      : failed ? <div className="agent-message-data-state"><p role="alert">无法读取这条消息的原始记录。</p><button type="button" onClick={() => setAttempt(value => value + 1)}>重试读取</button></div>
        : <p className="agent-message-data-state" role="status">正在读取原始数据…</p>}
  </dialog>, document.body);
}
