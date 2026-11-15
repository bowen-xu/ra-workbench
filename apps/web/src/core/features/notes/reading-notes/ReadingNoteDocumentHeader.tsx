import { History, MoreHorizontal, Trash2, Star } from "lucide-react";
import { isDependentReadingNote, type ReadingNoteBlock } from "@research-assistant/core/notes";
import { ControlHint } from "../../../components/ControlHint";
import { ActionMenu, ActionMenuItem } from "../../../components/ActionMenu";
import { HbsNoteOperationLogMenu } from "../../../components/hbs/HbsNoteOperationLogMenu";
import { NoteSaveRecoveryActions, type NoteSaveRecoveryActionsProps } from "../workspace/NoteSaveRecoveryActions";
import { NoteSaveState } from "../workspace/NoteSaveState";
import { useTextEditorHeld } from "../editor/noteTextEditorPorts";
import { ReadingNoteSourceSection } from "./ReadingNoteSourceSection";
import { NoteExportMenu } from "../transfer/NoteExportMenu";

interface ReadingNoteDocumentHeaderProps extends NoteSaveRecoveryActionsProps {
  readonly lifecycleActions?: ReactNode;
  readonly pageId?: string;
  readonly exportTitle?: string;
  readonly readOnly?: boolean;
  readonly onDiffControlsTarget?: (element: HTMLDivElement | null) => void;
  readonly note?: ReadingNoteBlock;
  readonly contextTitle?: string;
  readonly onColorChange?: (color: string) => void;
  readonly onFavoriteChange?: (favorite: boolean) => void;
  readonly favorite?: boolean;
  readonly saveMessage: string | undefined;
  readonly onOpenHistory: () => void;
  readonly onTrash: () => void;
}

export function ReadingNoteDocumentHeader(props: ReadingNoteDocumentHeaderProps): React.JSX.Element {
  const held = useTextEditorHeld(props.pageId ?? props.note?.pageId ?? "");
  const favorite = props.favorite ?? props.note?.isFavorite ?? false;
  return <>
    <div className="note-page-topline reading-note-document-topline">
      <div className="note-page-diff-controls" ref={props.onDiffControlsTarget} />
      <div className="reading-note-document-actions">
        {props.lifecycleActions}
        <NoteSaveRecoveryActions {...props} />
        <NoteSaveState status={props.saveStatus} message={props.saveMessage} {...(props.pageId ? { pageId: props.pageId } : props.note?.pageId ? { pageId: props.note.pageId } : {})} />
        {(!props.note || !isDependentReadingNote(props.note)) && props.onFavoriteChange && <ControlHint content={favorite ? "取消收藏" : "收藏笔记"}><button className="note-page-icon-button" aria-label={favorite ? "取消收藏" : "收藏笔记"} aria-pressed={favorite} disabled={held || props.readOnly || props.saveStatus === "conflict"} onClick={() => props.onFavoriteChange?.(!favorite)} type="button"><Star size={17} fill={favorite ? "currentColor" : "none"} /></button></ControlHint>}
        <ActionMenu hint className="note-page-menu" label={props.note ? "阅读笔记操作" : "笔记操作"} trigger={<MoreHorizontal aria-hidden="true" size={16} />} triggerClassName="note-page-icon-button" contentClassName="note-page-action-menu">
          <ActionMenuItem onClick={props.onOpenHistory}><History aria-hidden="true" size={16} />版本历史</ActionMenuItem>
          {(props.pageId ?? props.note?.pageId) && <NoteExportMenu pageId={(props.pageId ?? props.note?.pageId)!} title={props.exportTitle ?? props.contextTitle ?? "阅读笔记"}/>}
          <HbsNoteOperationLogMenu pageId={props.pageId ?? props.note?.pageId}/>
          <ActionMenuItem disabled={held || props.readOnly || props.saveStatus === "conflict"} className="danger-text" onClick={props.onTrash}><Trash2 aria-hidden="true" size={16} />移到回收站</ActionMenuItem>
        </ActionMenu>
      </div>
    </div>
    <header className="reading-note-document-header">
    <div className="reading-note-document-tools">
      {props.note ? <ReadingNoteSourceSection note={props.note} {...(props.onColorChange === undefined ? {} : { onColorChange: props.onColorChange })} /> : <p className="reading-note-source-paper">{props.contextTitle}</p>}
    </div>
  </header></>;
}
import type { ReactNode } from "react";
