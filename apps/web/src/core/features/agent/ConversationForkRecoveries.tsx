import { useEffect, useState } from "react";
import type { AgentConversationBinding, AgentConversationSummary } from "@research-assistant/core/agent";
import { getAppStorage } from "../../runtime/appStorage";
import { notify } from "../../services/notifications";
import { ConversationForkAction } from "./ConversationForkAction";
import { conversationForkInProgress, type ConversationForkPorts } from "./conversationForkController";
import { discoverConversationForks, forkScope, subscribeConversationForks, type ConversationForkRecord } from "./conversationForkPending";

interface Props {
  readonly binding: AgentConversationBinding; readonly ports: ConversationForkPorts;
  readonly visibleScopes: ReadonlySet<string>; readonly onOpen: (conversation: AgentConversationSummary) => void;
}
const pageSize = 16;
/** Independent of selected conversation, history availability and current CAS.
 * Only the existing original-intent owner may inspect, retry, read or finish. */
export function ConversationForkRecoveries({ binding, ports, visibleScopes, onOpen }: Props): React.JSX.Element | null {
  const identity = JSON.stringify(binding);
  let storage: Storage | null;
  try { storage = getAppStorage(); } catch { storage = null; }
  const [found, setFound] = useState<{ scope: string; storage: Storage; records: readonly ConversationForkRecord[] } | null>(null);
  const [limit, setLimit] = useState(pageSize);
  useEffect(() => {
    setLimit(pageSize);
    let alive = true;
    const report = (cause?: unknown) => {
      const code = cause instanceof Error && cause.message === "FORK_RECOVERY_INVALID" ? "FORK_RECOVERY_INVALID" : "FORK_RECOVERY_UNAVAILABLE";
      notify.error(`本机分支恢复记录不可读取；原记录已保留，请检查本机存储后重新核对。（${code}）`, { id: `fork-discovery:${identity}` });
    };
    const refresh = () => {
      if (!alive) return;
      if (!storage) { report(); return; }
      try { setFound({ scope: identity, storage, records: discoverConversationForks(storage, binding, report) }); }
      catch (cause: unknown) { report(cause); }
    };
    const detach = storage ? subscribeConversationForks(storage, binding, refresh) : () => {};
    refresh();
    return () => { alive = false; detach(); };
  }, [identity, storage]);
  const records = found?.scope === identity && found.storage === storage ? found.records.filter(record => !visibleScopes.has(forkScope(record.intent)) && !conversationForkInProgress(record.intent, storage)) : [];
  if (!records.length) return null;
  return <section className="conversation-fork-recoveries" aria-label="待核对的分支">
    <p>待核对的分支（{records.length}）</p>
    <p>可核对已保存的原请求；此处不会自动重试创建。</p>
    {records.slice(0, limit).map((record, index) => <div key={record.intent.childId} className="conversation-fork-control" role="group" aria-label={`待核对分支 ${index + 1}`}>
      <ConversationForkAction scope={record.intent} turnLabel={`待核对分支 ${index + 1}`} recoveryOnly disabled={false} ports={ports} onOpen={onOpen} />
    </div>)}
    {records.length > limit && <button type="button" onClick={() => setLimit(value => value + pageSize)}>显示更多分支核对记录</button>}
  </section>;
}
