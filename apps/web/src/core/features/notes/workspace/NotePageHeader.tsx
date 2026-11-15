import { useMemo, type ReactNode, type Ref } from "react";
import {
  Archive,
  ChevronRight,
  Copy,
  History,
  GitCompareArrows,
  Trash2,
} from "lucide-react";
import type { NotePage, NotePageId } from "@research-assistant/core/notes";

import { ActionMenuItem } from "../../../components/ActionMenu";
import { NoteExportMenu } from "../transfer/NoteExportMenu";
import { HbsNoteOperationLogMenu } from "../../../components/hbs/HbsNoteOperationLogMenu";
import type { NoteSaveStatus } from "../editor/noteSaveController";
import { NotePageToolbar } from "./NotePageToolbar";
import { useTextEditorHeld } from "../editor/noteTextEditorPorts";

export function notePageBreadcrumb(pages: readonly NotePage[], page: NotePage): readonly NotePage[] {
  const chain: NotePage[] = [];
  const visited = new Set<NotePageId>([page.id]);
  let parentId = page.parentPageId;
  while (parentId !== undefined && !visited.has(parentId)) {
    const parent = pages.find((candidate) => candidate.id === parentId);
    if (parent === undefined) break;
    chain.unshift(parent);
    visited.add(parent.id);
    parentId = parent.parentPageId;
  }
  return chain;
}

interface NotePageHeaderProps {
  readonly lifecycleActions?: ReactNode;
  readonly moreButtonRef?: Ref<HTMLButtonElement>;
  readonly onOpenRelations: () => void;
  readonly onDiffControlsTarget?: (element: HTMLDivElement | null) => void;
  readonly page: NotePage;
  readonly pages: readonly NotePage[];
  readonly saveStatus: NoteSaveStatus;
  readonly saveMessage: string | undefined;
  readonly onOpenPage: (id: NotePageId) => void;
  readonly onFavoriteChange: (favorite: boolean) => void;
  readonly onOpenHistory: () => void;
  readonly onDuplicate: () => void;
  readonly onArchive: () => void;
  readonly onTrash: () => void;
  readonly onRetrySave: () => void;
}

export function NotePageHeader(props: NotePageHeaderProps): React.JSX.Element {
  const held = useTextEditorHeld(props.page.id);
  const writing = held || props.saveStatus === "conflict" || props.page.status !== "active";
  const breadcrumbs = useMemo(() => notePageBreadcrumb(props.pages, props.page), [props.page, props.pages]);
  return <>
    <div className="note-page-topline">
      {breadcrumbs.length > 0 && <nav aria-label="页面路径" className="note-page-breadcrumbs">
        {breadcrumbs.map((page, index) => <span key={page.id}>
          {index > 0 && <ChevronRight aria-hidden="true" size={13} />}
          <button onClick={() => props.onOpenPage(page.id)} type="button">{page.title}</button>
        </span>)}
      </nav>}
      <div className="note-page-diff-controls" ref={props.onDiffControlsTarget} />
      <NotePageToolbar lifecycleActions={props.lifecycleActions} disabled={writing} onRetrySave={props.onRetrySave} pageId={props.page.id} saveStatus={props.saveStatus} saveMessage={props.saveMessage} favorite={props.page.isFavorite} onFavoriteChange={props.onFavoriteChange}
        {...(props.moreButtonRef === undefined ? {} : { moreButtonRef: props.moreButtonRef })}>
          <ActionMenuItem onClick={props.onOpenRelations}><GitCompareArrows aria-hidden="true" size={16} />出链与反链</ActionMenuItem>
          <ActionMenuItem onClick={props.onOpenHistory}><History aria-hidden="true" size={16} />版本历史</ActionMenuItem>
          <NoteExportMenu pageId={props.page.id} title={props.page.title}/>
          <HbsNoteOperationLogMenu pageId={props.page.id}/>
          <ActionMenuItem disabled={writing} onClick={props.onDuplicate}><Copy aria-hidden="true" size={16} />创建副本</ActionMenuItem>
          <ActionMenuItem disabled={writing} onClick={props.onArchive}><Archive aria-hidden="true" size={16} />归档</ActionMenuItem>
          <ActionMenuItem disabled={writing} className="danger-text" onClick={props.onTrash}><Trash2 aria-hidden="true" size={16} />移到回收站</ActionMenuItem>
      </NotePageToolbar>
    </div>
    </>;
}
