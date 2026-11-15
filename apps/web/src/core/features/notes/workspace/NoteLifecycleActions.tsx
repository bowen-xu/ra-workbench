import type { NotePage } from "@research-assistant/core/notes";
import { ArchiveRestore, Trash2 } from "lucide-react";
import { ControlHint } from "../../../components/ControlHint";
import { useTextEditorHeld } from "../editor/noteTextEditorPorts";

interface Props {
  readonly page: NotePage;
  readonly disabled?: boolean;
  readonly onRestore?: (() => void) | undefined;
  readonly onPermanentlyDelete?: (() => void) | undefined;
}

/** Lifecycle actions remain available while the document itself is read-only. */
export function NoteLifecycleActions({ page, disabled, onRestore, onPermanentlyDelete }: Props): React.JSX.Element | null {
  const held = useTextEditorHeld(page.id);
  if (page.status === "active") return null;
  return <>
    {onRestore && <ControlHint content="恢复笔记"><button aria-label="恢复笔记" className="note-page-icon-button" disabled={disabled || held} onClick={onRestore} type="button"><ArchiveRestore aria-hidden="true" size={17} /></button></ControlHint>}
    {page.status === "trashed" && onPermanentlyDelete && <ControlHint content="永久删除"><button aria-label="永久删除" className="note-page-icon-button note-lifecycle-delete" disabled={disabled || held} onClick={onPermanentlyDelete} type="button"><Trash2 aria-hidden="true" size={17} /></button></ControlHint>}
  </>;
}
