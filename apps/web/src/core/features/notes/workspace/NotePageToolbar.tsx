import type { ReactNode, Ref } from "react";
import { MoreHorizontal, SlidersHorizontal, Star } from "lucide-react";
import { ActionMenu } from "../../../components/ActionMenu";
import { ControlHint } from "../../../components/ControlHint";
import type { NoteSaveStatus } from "../editor/noteSaveController";
import { NoteSaveRecoveryActions } from "./NoteSaveRecoveryActions";
import { NoteSaveState } from "./NoteSaveState";
import "./NotePageToolbar.css";
import { useTextEditorHeld } from "../editor/noteTextEditorPorts";

interface Props {
  readonly lifecycleActions?: ReactNode;
  readonly pageId?: string;
  readonly disabled?: boolean;
  readonly onRetrySave?: () => void;
  readonly saveStatus: NoteSaveStatus;
  readonly saveMessage: string | undefined;
  readonly favorite: boolean;
  readonly canFavorite?: boolean;
  readonly onFavoriteChange: (favorite: boolean) => void;
  /** Optional file-details surface for writing files; ordinary Note Pages have no properties panel. */
  readonly propertiesId?: string;
  readonly showProperties?: boolean;
  readonly onToggleProperties?: () => void;
  readonly moreButtonRef?: Ref<HTMLButtonElement>;
  readonly children: ReactNode;
}

export function NotePageToolbar(props: Props): React.JSX.Element {
  const held = useTextEditorHeld(props.pageId ?? "");
  const writing = held || !!props.disabled || props.saveStatus === "conflict";
  const favoriteLabel = props.favorite ? "取消收藏" : "收藏页面";
  return <div className="note-page-toolbar" role="toolbar" aria-label="笔记页面操作">
    {props.lifecycleActions}
    {props.onRetrySave && <NoteSaveRecoveryActions saveStatus={props.saveStatus} saveMessage={props.saveMessage} onRetrySave={props.onRetrySave} />}
    <NoteSaveState status={props.saveStatus} message={props.saveMessage} {...(props.pageId ? { pageId: props.pageId } : {})} />
    {props.canFavorite !== false && <ControlHint content={favoriteLabel}><button disabled={writing} aria-label={favoriteLabel} aria-pressed={props.favorite} className="note-page-icon-button" onClick={() => props.onFavoriteChange(!props.favorite)} type="button"><Star aria-hidden="true" fill={props.favorite ? "currentColor" : "none"} size={17} /></button></ControlHint>}
    {props.onToggleProperties && <ControlHint content="页面属性"><button aria-label="页面属性" aria-controls={props.propertiesId} aria-expanded={props.showProperties} className="note-page-icon-button" onClick={props.onToggleProperties} type="button"><SlidersHorizontal aria-hidden="true" size={17} /></button></ControlHint>}
    <ActionMenu hint portal className="note-page-menu" contentClassName="note-page-action-menu" label="更多页面操作" trigger={<MoreHorizontal aria-hidden="true" size={18} />} triggerClassName="note-page-icon-button" {...(props.moreButtonRef === undefined ? {} : { triggerRef: props.moreButtonRef })}>{props.children}</ActionMenu>
  </div>;
}
