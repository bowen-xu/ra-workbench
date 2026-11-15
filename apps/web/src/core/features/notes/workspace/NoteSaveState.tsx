import "./NotePageToolbar.css";
import { Check, CircleAlert, Clock, LoaderCircle } from "lucide-react";
import { ControlHint } from "../../../components/ControlHint";
import type { NoteSaveStatus } from "../editor/noteSaveController";
import { noteSaveLabel } from "./notesPresentation";
import { useNoteReviewMenuActions,useTextEditorHeld } from "../editor/noteTextEditorPorts";
import { NoteRecoveryMenu } from "./NoteRecoveryMenu";

export function NoteSaveState({ status, message, pageId }: {
  readonly status: NoteSaveStatus;
  readonly message: string | undefined;
  readonly pageId?: string;
}): React.JSX.Element {
  const writing = useTextEditorHeld(pageId ?? "");
  const actions=useNoteReviewMenuActions(pageId);
  const effectiveStatus = writing ? "saving" : status;
  const recoveryError = effectiveStatus === "failed";
  const Icon = { saved: Check, dirty: Clock, saving: LoaderCircle, failed: CircleAlert, conflict: CircleAlert }[effectiveStatus];
  const proposalRecovery=message?.includes("MODEL_NOTE_PROPOSAL_STALE");
  const label = writing ? "agent正在写入" : `${noteSaveLabel(status)}${message === undefined ? "" : `：${message}`}${status === "conflict" && !recoveryError && !proposalRecovery ? "；请在正文内使用 Undo／Keep、Take／Both 处理" : ""}`;
  // Keep this page's recovery in its existing right-hand status menu.
  return <>{pageId && !writing && (recoveryError||effectiveStatus==="conflict"||actions.length>0) ? <NoteRecoveryMenu pageId={pageId} label={label} status={effectiveStatus} message={message} actions={actions}/> : <ControlHint content={label}>
    <span aria-label={label} className={`note-save-state ${effectiveStatus}`} role="status" tabIndex={0}>
      <Icon aria-hidden="true" size={16} />
    </span>
  </ControlHint>}</>;
}
