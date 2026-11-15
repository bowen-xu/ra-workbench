import { ControlHint } from "../../../public/overlay";
import { Maximize2, Pause, Play, Trash2 } from "lucide-react";
import { useState } from "react";
import { Dialog } from "../../../public/overlay";
import type { WritingInputQueueProps } from "./WritingInputQueue";
import { WritingInputQueue } from "./WritingInputQueue";
import "./WritingAgentTaskStack.css";

export type WritingAgentGoalState = "idle" | "running" | "paused";

export interface WritingAgentGoal {
  readonly text: string;
  readonly state: WritingAgentGoalState;
  readonly onDelete?: () => void;
  readonly onToggle?: () => void;
  readonly onEdit?: (text: string) => void;
  readonly statusDescription?: string;
}

export interface WritingAgentTaskStackProps {
  /** The queue is supplied by either the durable runtime or a mock gateway. */
  readonly queue?: WritingInputQueueProps;
  /** The active goal is supplied by either the durable runtime or a mock gateway. */
  readonly goal?: WritingAgentGoal;
}

function goalStateLabel(state: WritingAgentGoalState): string {
  return state === "paused" ? "暂停目标" : "追求目标";
}

function WritingAgentGoalRow({ goal }: { readonly goal: WritingAgentGoal }): React.JSX.Element {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(goal.text);
  const canEdit = goal.onEdit !== undefined;

  function openEditor(): void {
    setDraft(goal.text);
    setEditing(true);
  }

  function closeEditor(): void {
    setEditing(false);
    setDraft(goal.text);
  }

  function saveEditor(): void {
    const next = draft.trim();
    if (!next || !goal.onEdit) return;
    goal.onEdit(next);
    setEditing(false);
  }

  return <>
    <div className="writing-agent-goal" data-goal-state={goal.state}>
      <span aria-hidden="true" className={`writing-agent-goal-status-dot${goal.state === "running" ? " running" : ""}`} />
      <strong>{goalStateLabel(goal.state)}</strong>
      <ControlHint content={goal.text}><span className="writing-agent-goal-label">{goal.text}</span></ControlHint>
      <div className="writing-agent-goal-actions">
        {goal.onDelete && <button aria-label="删除目标" className="writing-agent-goal-action" onClick={goal.onDelete}  type="button"><Trash2 aria-hidden="true" size={15} /></button>}
        {goal.onToggle && <button aria-label={goal.state === "running" ? "暂停目标" : "开始目标"} className="writing-agent-goal-action" onClick={goal.onToggle}  type="button">{goal.state === "running" ? <Pause aria-hidden="true" size={15} /> : <Play aria-hidden="true" size={15} />}</button>}
        {canEdit && <button aria-label="编辑目标" className="writing-agent-goal-action" onClick={openEditor}  type="button"><Maximize2 aria-hidden="true" size={15} /></button>}
      </div>
    </div>
    {editing && <Dialog open portal backdrop={false} hideOnInteractOutside={false} onClose={closeEditor}
      aria-labelledby="writing-agent-goal-dialog-title" className="writing-agent-goal-modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) closeEditor(); }}>
      <div className="writing-agent-goal-modal">
        <h2 id="writing-agent-goal-dialog-title">编辑目标</h2>
        <label htmlFor="writing-agent-goal-editor">目标内容</label>
        <input id="writing-agent-goal-editor" aria-label="目标内容" autoFocus value={draft} onChange={(event) => setDraft(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) saveEditor(); }} />
        {goal.statusDescription && <p>{goal.statusDescription}</p>}
        <div className="writing-agent-goal-modal-actions">
          <button type="button" onClick={closeEditor}>取消</button>
          <button type="button" disabled={!draft.trim()} onClick={saveEditor}>保存</button>
        </div>
      </div>
    </Dialog>}
  </>;
}

/**
 * Shared queue/goal presentation used by the real writing runtime and its
 * mock-data preview. Only the queue and goal gateways differ between them.
 */
export function WritingAgentTaskStack({ queue, goal }: WritingAgentTaskStackProps): React.JSX.Element | null {
  const state=queue?.state, compaction=queue?.compaction;
  const executing=compaction?.state === "pending" ? compaction.inputId ?? compaction.requestId : undefined;
  const showQueue=!!queue && !!state && (state.error !== null || state.uncertain
    || !!state.queue?.inputs.some(input=>input.inputId !== executing && !queue.hiddenInputIds?.includes(input.inputId)
      && !(queue.preparingStatus === "sent" && input.inputId === state.queue?.preparingInputId)));
  if (!showQueue && !goal) return null;
  return <div className={`writing-agent-task-stack${showQueue ? " has-queue" : ""}${goal ? " has-goal" : ""}`}>
    {showQueue && queue && <WritingInputQueue {...queue} />}
    {goal && <WritingAgentGoalRow goal={goal} />}
  </div>;
}
