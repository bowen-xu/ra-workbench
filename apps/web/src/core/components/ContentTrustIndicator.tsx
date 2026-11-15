import type { ContentTrustNode, ContentTrustStatus } from "@research-assistant/core/content-trust";
import {
  confirmableContentTrustIds,
  nextPendingContentTrustNode,
  summarizeContentTrust,
} from "@research-assistant/core/content-trust";

import "./ContentTrustIndicator.css";
import { ControlHint } from "./ControlHint";

export const CONTENT_TRUST_LABELS: Readonly<Record<ContentTrustStatus, string>> = {
  untrusted: "不可信：AI 生成，尚未人工确认",
  reviewable: "较可信：自动提取或人工编辑，尚未确认",
  trusted: "可信：已经人工确认",
  neutral: "无可审查内容",
};

interface ContentTrustIndicatorProps {
  readonly status: ContentTrustStatus;
  readonly subjectLabel?: string;
  readonly statusLabel?: string;
  readonly onActivate?: (() => void) | undefined;
  readonly size?: "small" | "medium";
  readonly showLabel?: boolean;
  readonly showHint?: boolean;
  readonly className?: string;
}

export function ContentTrustIndicator({
  status,
  subjectLabel,
  statusLabel: suppliedLabel,
  onActivate,
  size = "medium",
  showLabel = false,
  showHint = true,
  className = "",
}: ContentTrustIndicatorProps): React.JSX.Element {
  const statusLabel = suppliedLabel ?? CONTENT_TRUST_LABELS[status];
  const accessibleLabel = subjectLabel === undefined ? statusLabel : `${subjectLabel}：${statusLabel}`;
  const classes = `content-trust-indicator ${status} ${size} ${showLabel ? "with-label" : ""} ${className}`.trim();
  const contents = (
    <>
      <span aria-hidden="true" className="content-trust-dot" />
      {showLabel && <span className="content-trust-label">{statusLabel}</span>}
    </>
  );
  if (onActivate !== undefined) {
    const control = (
      <button aria-label={accessibleLabel} className={classes} onClick={onActivate} type="button">
        {contents}
      </button>
    );
    return <ControlHint content={showHint ? accessibleLabel : undefined}>{control}</ControlHint>;
  }
  const control = (
    <span aria-label={accessibleLabel} className={classes} role="img">
      {contents}
    </span>
  );
  return <ControlHint content={showHint ? accessibleLabel : undefined}>{control}</ControlHint>;
}

interface ContentTrustGroupControlsProps {
  readonly tree: ContentTrustNode;
  readonly currentId?: string | undefined;
  readonly onConfirmPending?: ((ids: readonly string[]) => void) | undefined;
  readonly onNavigate?: ((node: ContentTrustNode) => void) | undefined;
  readonly compact?: boolean;
  readonly hideResolvedIndicator?: boolean;
}

export function ContentTrustGroupControls({
  tree,
  currentId,
  onConfirmPending,
  onNavigate,
  compact = false,
  hideResolvedIndicator = false,
}: ContentTrustGroupControlsProps): React.JSX.Element {
  const summary = summarizeContentTrust(tree);
  const ids = confirmableContentTrustIds(tree);
  const next = nextPendingContentTrustNode(tree, currentId);
  return (
    <div aria-label={`${tree.label}可信状态操作`} className={`content-trust-group-controls ${compact ? "compact" : ""}`}>
      {(!hideResolvedIndicator || summary.status === "untrusted" || summary.status === "reviewable") && (
        <ContentTrustIndicator showLabel={!compact} status={summary.status} subjectLabel={tree.label} />
      )}
      {!compact && summary.pendingCount > 0 && (
        <span className="content-trust-counts">
          {summary.untrustedCount > 0 && <span>红 {summary.untrustedCount}</span>}
          {summary.reviewableCount > 0 && <span>黄 {summary.reviewableCount}</span>}
        </span>
      )}
      {next !== undefined && onNavigate !== undefined && (
        <button className="content-trust-text-action" onClick={() => onNavigate(next)} type="button">
          下一个待确认
        </button>
      )}
      {ids.length > 0 && onConfirmPending !== undefined && (
        <button className="content-trust-text-action primary" onClick={() => onConfirmPending(ids)} type="button">
          确认本组（{ids.length}）
        </button>
      )}
    </div>
  );
}
