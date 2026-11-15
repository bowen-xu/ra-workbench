import { useEffect, useState } from "react";
import type { ConversationDeletionRecord } from "./conversationDeletionPending";

/** Opens the production confirmation/recovery surface; it never calls a gateway. */
export function ConversationDeletionRecoveries({records, issue, activeId, showOrigin = false, onOpen}: {
  readonly records: readonly ConversationDeletionRecord[]; readonly issue: string | null;
  readonly activeId?: string | null;
  readonly showOrigin?: boolean;
  readonly onOpen: (record: ConversationDeletionRecord) => void;
}): React.JSX.Element | null {
  const visible = records.filter(record => record.intent.conversationId !== activeId);
  const [limit, setLimit] = useState(16);
  useEffect(() => {setLimit(16);}, [visible.length]);
  if (!visible.length && !issue) return null;
  return <section className="conversation-deletion-recoveries" aria-label="待核对的删除">
    {issue && <p role="alert">本机删除恢复记录不可读取，原记录已保留。（{issue}）</p>}
    {visible.length > 0 && <><p>待核对的删除（{visible.length}）</p>
      {visible.slice(0, limit).map(record => <button type="button" key={record.intent.requestId}
        aria-label={`核对“${record.intent.expectedTitle}”`} onClick={() => onOpen(record)}>核对“{record.intent.expectedTitle}”
        {showOrigin && record.intent.binding && <small>原{{manuscript: "稿件", paper: "文献", note: "笔记", readingNote: "阅读笔记"}[record.intent.binding.kind]}</small>}
      </button>)}
      {visible.length > limit && <button type="button" onClick={() => setLimit(value => value + 16)}>显示更多删除核对记录</button>}
    </>}
  </section>;
}
