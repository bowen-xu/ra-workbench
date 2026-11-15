import {useCallback, useEffect, useMemo, useState} from "react";
import type {HbsSession} from "@research-assistant/core/hbs";
import type {NoteDocumentV1} from "@research-assistant/core/notes";
import type {NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {useDeveloperMode} from "../../features/settings/DeveloperMode";
import {HbsOperationLog} from "./operationLog";
import {captureNoteOperations} from "./noteOperationCapture";
import {registerNoteOperationLogActions, type NoteOperationLogActions} from "./noteOperationLogActions";
import {useOperationLogExport} from "./useOperationLogExport";
import {notify} from "../../services/notifications";

/** Retained production editors own their diagnostic buffer. Developer mode is
 * the capture boundary, not merely a visibility switch for already-kept data. */
export function HbsNoteOperationLog({editor, session}: {
  readonly editor: NoteEditorInstance;
  readonly session: HbsSession<NoteDocumentV1>;
}): null {
  const {enabled} = useDeveloperMode();
  const log = useMemo(() => new HbsOperationLog({scope: "note-editor"}), [session]);
  const [generation, restart] = useState(0);
  const {copy, download, copying, downloading} = useOperationLogExport(log);
  const clear = useCallback(() => {
    log.clear({forgetSessions: true}); restart(value => value + 1);
    notify.success("已清空操作记录");
  }, [log]);
  const actions = useMemo<NoteOperationLogActions>(() => ({copy, download, clear, copying, downloading}),
    [copy, download, clear, copying, downloading]);
  const pageId = session.getSnapshot().content.pageId;
  useEffect(() => {
    if (!enabled) {log.clear({forgetSessions: true}); return;}
    return captureNoteOperations(log, editor, session);
  }, [enabled, editor, session, log, generation]);
  useEffect(() => {
    if (enabled) return registerNoteOperationLogActions(pageId, actions);
  }, [enabled, pageId, actions]);
  useEffect(() => () => log.clear({forgetSessions: true}), [log]);
  return null;
}
