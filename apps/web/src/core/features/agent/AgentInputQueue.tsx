import { ReorderableListRow, ReorderDragHandle } from "../../components/ReorderableListRow";
import { usePointerReorder } from "../../components/reorder/usePointerReorder";
import { ReorderGroup } from "../../components/reorder/ReorderGroup";
import { ControlHint } from "../../components/ControlHint";
import { Copy, CornerUpRight, ListPlus, MoreHorizontal, Pencil, RotateCw, Trash2 } from "lucide-react";
import { notify } from "../../services/notifications";
import { ShimmerText } from "../../components/ShimmerText";
import { useEffect, useRef } from "react";
import { ActionMenu, ActionMenuItem } from "../../components/ActionMenu";
import type { AgentQueueEdit, AgentQueueState } from "@research-assistant/core/agent";
import type { AgentInputQueueState } from "@research-assistant/data/agent";
import "./AgentInputQueue.css";
import type { AgentCompactionRecord } from "./AgentCompactionActivity";
import { queueReadDiagnostic } from "./queueReadDiagnostic";
import { copyText } from "./copyText";

export interface AgentInputQueueProps {
  readonly state: AgentInputQueueState<AgentQueueState>;
  readonly className?: string;
  readonly explanation?: string;
  readonly disabled: boolean;
  readonly compaction?: AgentCompactionRecord | undefined;
  readonly onEdit: (edit: AgentQueueEdit) => Promise<boolean>;
  readonly onEditInComposer?: (input: AgentQueueState["inputs"][number], index: number, queue: AgentQueueState) => Promise<boolean>;
  readonly onReload: () => void | Promise<void>;
  readonly onRetry: () => void;
  /** Dispatch into the running turn, or explicitly start a new turn when idle. */
  readonly onSteer?: (inputId: string, text: string) => Promise<boolean>;
  /** Permit steering other rows while queue editing is separately protected. */
  readonly steerDisabled?: boolean | undefined;
  readonly steerStartsTurn?: boolean;
  /** The state of this exact queue preparation, when an owner can prove it. */
  readonly preparingStatus?: "none" | "sending" | "sent" | "recovery";
  /** Show the queue-level paused marker. */
  readonly showPausedLabel?: boolean;
  /** Items owned by a visible queue-to-steer handoff remain durable but are not queued for display. */
  readonly hiddenInputIds?: readonly string[] | undefined;
}
const originLabel: Record<string, string> = { manuscript: "稿件", paper: "文献", note: "独立笔记", readingNote: "阅读笔记" };
export function AgentInputQueue({ state, disabled, compaction, onEdit, onEditInComposer, onReload, onRetry, onSteer, steerDisabled, steerStartsTurn=false, preparingStatus="recovery", hiddenInputIds=[], className="agent-input-queue" }: AgentInputQueueProps): React.JSX.Element | null {
  const compactionId = compaction?.state === "pending" ? compaction.id : undefined;
  const executingInputId = compactionId ? compaction?.inputId ?? compaction?.requestId : undefined;
  const queue = state.queue;
  const visibleInputs = queue?.inputs.filter(input => input.inputId !== executingInputId && !hiddenInputIds.includes(input.inputId)
    && !(preparingStatus === "sent" && input.inputId === queue.preparingInputId)) ?? [];
  const recoveryActions = useRef({ onRetry, onReload });
  recoveryActions.current = { onRetry, onReload };
  useEffect(() => {
    if (!state.error || state.editing) return;
    const isReadFailure = state.error === "读取队列失败，请重试。";
    const diagnostic = isReadFailure && state.readFailure ? queueReadDiagnostic(state.readFailure) : state.error;
    notify.error(state.error, isReadFailure ? {
      actions: [
        { id: "copy", label: "复制错误信息", icon: <Copy aria-hidden="true" />, onClick: async () => {
          if (await copyText(diagnostic)) notify.success("已复制错误信息"); else notify.error("复制错误信息失败，请重试。");
        } },
        { id: "reload", label: "重试", icon: <RotateCw aria-hidden="true" />, onClick: () => recoveryActions.current.onReload() },
      ],
    } : undefined);
  }, [state.error, state.editing, state.uncertain, state.readFailure]);
  const busy = disabled || state.loading || state.editing || state.uncertain || hiddenInputIds.length > 0;
  const steerBusy = (steerDisabled ?? disabled) || state.loading || state.editing || state.uncertain || !!state.pending || !!state.error;
  const reorder = usePointerReorder({ids:visibleInputs.map(input=>input.inputId),
    disabled:busy || !!executingInputId || !!queue?.preparingInputId || hiddenInputIds.length > 0,
    onReorder:({order})=>onEdit({type:"reorder",inputIds:order})});
  const draggingInputId=reorder.drag?.id;
  const orderedInputs=reorder.order.flatMap(id=>visibleInputs.filter(input=>input.inputId===id));
  const hasQueueActivity = state.error !== null || state.uncertain || visibleInputs.length > 0;
  if (!hasQueueActivity) return null;
  return <section aria-label="队列消息" className={`${className} agent-input-queue-card${visibleInputs.length > 1 ? " agent-input-queue-can-reorder" : ""}${draggingInputId ? " is-reordering" : ""}`}>
    {state.retrying && <p className="agent-input-queue-status" role="status"><ShimmerText>正在重新连接…</ShimmerText></p>}
    {state.uncertain && !state.editing && <div className="agent-input-queue-recovery" role="status">
      <span>队列暂时无法同步</span>
      <button type="button" aria-label="重试队列同步" disabled={disabled} onClick={onRetry}><RotateCw size={14}/></button>
    </div>}
    {queue && <div className="agent-input-queue-body">
      <ReorderGroup as="ol" order={reorder.order} dragging={!!reorder.drag}>{orderedInputs.map((input, index) => <ReorderableListRow key={input.inputId} itemId={input.inputId}
        ref={row => { if(row)reorder.rows.current.set(input.inputId,row);else reorder.rows.current.delete(input.inputId); }}
        dragging={draggingInputId===input.inputId} offset={reorder.offsetFor(input.inputId)}>
        <div className="agent-input-queue-item-heading">
          {visibleInputs.length > 1 ? <ReorderDragHandle compact={className.split(/\s+/u).includes("writing-agent-input-queue")} className="agent-input-queue-drag-handle" aria-label={`拖动调整第 ${index + 1} 条消息顺序`} draggable={false}
            onPointerDown={event=>reorder.start(input.inputId,event)}
            disabled={busy || reorder.pending || !!executingInputId || !!queue.preparingInputId || hiddenInputIds.length > 0 || (!!draggingInputId && draggingInputId!==input.inputId)} /> : <span aria-hidden="true" className="agent-input-queue-drag-placeholder" />}
          <ListPlus aria-hidden="true" className="agent-input-queue-item-icon" size={17} />
          <ControlHint content={`入队来源：${input.origin ? `${originLabel[input.origin.kind]} · ${input.origin.resourceId}` : "未记录"}`}><p aria-description={`入队来源：${input.origin ? `${originLabel[input.origin.kind]} · ${input.origin.resourceId}` : "未记录"}`} className="ui-fade-truncated">{input.text}</p></ControlHint>
          <div className="agent-input-queue-item-actions">
            {onSteer && <ControlHint content="（非打断）提交引导"><button type="button" className="agent-input-queue-inline-action" aria-label={steerStartsTurn ? `用第 ${index + 1} 条消息启动新轮次` : `将第 ${index + 1} 条消息插入当前轮次`}  disabled={steerBusy || !!compactionId || queue.preparingInputId === input.inputId} onClick={() => { if (!compactionId) void onSteer(input.inputId, input.text); }}><CornerUpRight aria-hidden="true" size={14} /><span>引导</span></button></ControlHint>}
            <button type="button" className="agent-input-queue-inline-action agent-input-queue-icon-action" aria-label={`移除第 ${index + 1} 条消息`}  disabled={busy || queue.preparingInputId === input.inputId} onClick={() => void onEdit({ type: "remove", inputId: input.inputId })}><Trash2 aria-hidden="true" size={15} /></button>
            <ActionMenu showTitle={false} label={`第 ${index + 1} 条消息菜单`} trigger={<MoreHorizontal aria-hidden="true" size={16} />} className="agent-input-queue-item-menu-shell" triggerClassName="agent-input-queue-item-more" contentClassName="agent-input-queue-menu">
              <ActionMenuItem disabled={busy || !onEditInComposer || queue.preparingInputId === input.inputId} onClick={() => void onEditInComposer?.(input, queue.inputs.findIndex(item => item.inputId === input.inputId), queue)}><Pencil aria-hidden="true" size={14} />编辑消息</ActionMenuItem>
            </ActionMenu>
          </div>
        </div>
      </ReorderableListRow>)}</ReorderGroup>
    </div>}
  </section>;
}
