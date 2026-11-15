import { FileQuestionMark, FileWarning, LoaderCircle, RotateCw, ArrowLeft } from "lucide-react";
import type {NoteDocumentPendingProps} from "@research-assistant/workbench-sdk/note-presentation";
import { useDeveloperMode } from "../../settings/DeveloperMode";

/** Selection responds before the target's durable editor owner is ready. */
export function NoteDocumentPending({title, pageId, failure, onRetry, onHome}: NoteDocumentPendingProps): React.JSX.Element {
  const missing = failure?.kind === "missing";
  const { enabled: developerMode } = useDeveloperMode();
  const Icon = failure ? missing ? FileQuestionMark : FileWarning : LoaderCircle;
  return <section className={`notes-document-pending note-empty-state${failure ? " is-failed" : " is-loading"}`} aria-label="笔记加载状态">
    <Icon className={failure ? undefined : "spinning"} size={40} strokeWidth={1.5} aria-hidden="true" />
    <h1>{failure ? missing ? "Oops，找不到这篇笔记" : "笔记暂时无法打开" : title || "打开笔记"}</h1>
    {failure ? <>
      <p>{missing ? "链接对应的笔记不存在。你可以返回笔记，选择其他页面，或重新加载再试。" : "读取或准备笔记时遇到了问题，请重新加载。"}</p>
      {title && <p className="notes-document-target">{title}</p>}
      <div className="notes-document-pending-actions">
        {onHome && <button className={missing ? "primary-button" : "notes-document-retry"} type="button" onClick={onHome}><ArrowLeft size={15} aria-hidden="true" />返回笔记</button>}
        <button className={onHome && missing ? "notes-document-retry" : "primary-button"} type="button" onClick={onRetry}><RotateCw size={15} aria-hidden="true" />重新加载笔记</button>
      </div>
      {developerMode && <details className="notes-document-diagnostics"><summary>查看错误详情</summary><pre>{`${failure.code}\n笔记 ID：${pageId}\n${failure.message}`}</pre></details>}
    </> : <p role="status">正在打开笔记…</p>}
  </section>;
}
