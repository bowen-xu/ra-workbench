import type {ComponentProps, ReactNode} from "react";
import {ActionMenu, ActionMenuItem} from "../ActionMenu";
import "./NoteActionMenu.css";

/** Note commands share presentation; the common adapter owns every overlay. */
export function NoteActionMenu(props: ComponentProps<typeof ActionMenu>): React.JSX.Element {
  return <ActionMenu {...props} contentClassName={`note-action-menu ${props.contentClassName ?? ""}`}/>;
}
export function NoteActionMenuItem(props: ComponentProps<typeof ActionMenuItem>): React.JSX.Element {
  return <ActionMenuItem {...props} className={`note-action-menu-item ${props.className ?? ""}`}/>;
}
export function NoteActionMenuGroup({label, children}: {readonly label: string; readonly children: ReactNode}): React.JSX.Element {
  return <div role="group" aria-label={label} className="note-action-menu-group">
    <div className="note-action-menu-group-heading" aria-hidden="true">{label}</div>
    {children}
  </div>;
}
