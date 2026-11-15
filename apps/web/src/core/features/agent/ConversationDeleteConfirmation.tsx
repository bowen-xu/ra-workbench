import { useCallback, useEffect, useRef, useState } from "react";
import { OverlayPortal } from "../../components/overlay/OverlayPortal";
import type { ConversationDeleteConfirmationPreferences } from "./ConversationDeleteDialog";

interface Props {
  readonly pending: boolean;
  readonly blocked: boolean;
  readonly retry: boolean;
  readonly preferences?: ConversationDeleteConfirmationPreferences;
  readonly autoConfirm?: boolean;
  readonly actionLabel?: string;
  readonly errorMessage?: string;
  readonly feedbackTone?: "error" | "status";
  readonly secondaryAction?: {readonly label: string; readonly onSelect: () => void};
  readonly onClose: () => void;
  readonly onConfirm: () => void;
}

export function ConversationForceDeleteDialog(props: Props): React.JSX.Element {
  return <ConversationDeleteConfirmation {...props} title="强制删除对话" tone="warning"
    description="强制删除会清理对话历史、分支来源、执行副本及其清理记录，操作无法撤销。文件内容、待确认修改及文件恢复记录会保留。正在运行的对话需先停止。" />;
}

/** Each confirmation owns its modal, checkbox and initial cancel focus independently. */
export function ConversationDeleteConfirmation({ pending, blocked, retry, preferences, autoConfirm = true, actionLabel, feedbackTone = "error", secondaryAction, onClose, onConfirm,
  title = "永久删除对话", description, tone = "neutral", confirmLabel = "确认", errorMessage }: Props & {
    readonly title?: string; readonly description?: string; readonly tone?: "neutral" | "warning";
    readonly confirmLabel?: string;
  }): React.JSX.Element {
  const dialog = useRef<HTMLDialogElement>(null);
  const [surface, setSurface] = useState<HTMLDialogElement | null>(null);
  const started = useRef(false);
  const attach = useCallback((element: HTMLDialogElement | null) => {
    if (dialog.current !== element && dialog.current?.open) dialog.current.close();
    dialog.current = element; setSurface(element);
  }, []);
  const [skipToday, setSkipToday] = useState(() => preferences?.shouldSkip() ?? false);
  useEffect(() => {
    if (!surface) return;
    if (!started.current) {
      started.current = true;
      if (skipToday && autoConfirm && !retry) { onConfirm(); return; }
    }
    if (skipToday && autoConfirm && !retry) return;
    if (!surface.open) {
      surface.showModal(); surface.querySelector<HTMLButtonElement>("button")?.focus();
    }
    const focusCancel = () => {
      const active = surface.ownerDocument.activeElement;
      if (surface.isConnected && surface.open && (active === surface || active === surface.ownerDocument.body))
        surface.querySelector<HTMLButtonElement>("button")?.focus();
    };
    focusCancel();
    // Portal/native modal replacement may finish restoring focus after this effect.
    const frame = requestAnimationFrame(focusCancel);
    return () => cancelAnimationFrame(frame);
  }, [surface, retry, autoConfirm, pending]);
  return <OverlayPortal kind="surface"><dialog ref={attach} className={`conversation-delete-dialog${tone === "warning" ? " conversation-delete-dialog-warning" : ""}`} aria-label={title} aria-busy={pending}
    onCancel={(event) => { event.preventDefault(); if (!pending) onClose(); }}>
    <h2>{title}</h2>
    {description && <p className="conversation-delete-description">{description}</p>}
    {retry && !pending && <p className={`conversation-delete-description ${feedbackTone === "status" ? "conversation-delete-status" : "conversation-delete-error"}`} role={feedbackTone === "status" ? "status" : "alert"}>
      {errorMessage ?? (blocked ? "暂时无法删除，关闭弹窗后会自动核对列表，请核对后再试。" : "删除失败或结果不明，请重试。")}
    </p>}
    {preferences && <label className="conversation-delete-skip"><input type="checkbox" checked={skipToday} disabled={pending}
      onChange={(event) => {
        const checked = event.target.checked;
        setSkipToday(checked);
        preferences.setSkipToday(checked);
      }} /><span>今日不再提示</span></label>}
    <div className="conversation-delete-actions">
      <button type="button" className="ghost-button" disabled={pending} onClick={onClose}>取消</button>
      {secondaryAction && <button type="button" className="ghost-button" disabled={pending} onClick={secondaryAction.onSelect}>{secondaryAction.label}</button>}
      <button type="button" className="primary-button" disabled={pending || blocked}
        onClick={onConfirm}>{pending ? "正在删除…" : actionLabel ?? (retry ? "重试" : confirmLabel)}</button>
    </div>
  </dialog></OverlayPortal>;
}
