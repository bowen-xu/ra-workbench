import { useEffect, useRef, type RefObject } from "react";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteSaveCoordinator } from "../editor/noteSaveController";
import { checkTextSnapshotReady, getNoteReviewSaveState, textInstallPort } from "../editor/noteTextEditorPorts";

interface PreparedNavigationSession {
  dispatch(): void; commit(snapshot: NoteDocumentSnapshot): Promise<void> | void; cancel(): void; recover(): void;
}
type Prepare = (before: NoteDocumentSnapshot, after: NoteDocumentV1) => PreparedNavigationSession;
// Mounted hosts own these admissions; removing a host removes its port.
const sessions = new Map<string, Set<Prepare>>();
export function prepareNavigationSessions(before: NoteDocumentSnapshot, after: NoteDocumentV1): PreparedNavigationSession | undefined {
  const ports = sessions.get(before.page.id);
  if (!ports?.size) return undefined;
  if (ports.size !== 1) throw new Error("笔记存在多个编辑会话，请关闭重复会话后重试。");
  return ports.values().next().value!(before, after);
}
export function useNoteNavigationSession(pageId: string | undefined, options: {
  readonly saves: RefObject<NoteSaveCoordinator | undefined>;
  readonly getSnapshot: () => NoteDocumentSnapshot | undefined;
}): void {
  const latest = useRef(options); latest.current = options;
  useEffect(() => {
    if (!pageId) return;
    const prepare: Prepare = (before, after) => {
      const current = latest.current.getSnapshot(), saves = latest.current.saves.current;
      const review = getNoteReviewSaveState(pageId);
      if (!current || !saves || current.page.id !== before.page.id || current.page.revision !== before.page.revision || review.pendingChanges || review.status !== "saved")
        throw new Error("请先保存并完成修改审阅，再移动页面链接。");
      checkTextSnapshotReady(pageId);
      const installer = textInstallPort(pageId);
      const install = installer.prepareStructure(installer.read(), after);
      const lease = saves.beginExternalChange(before.page.revision, before.document);
      if (!lease) throw new Error("笔记保存会话正忙，请稍后重试。");
      return {
        dispatch() { if (!lease.markDispatched()) throw new Error("笔记内容已变化，请重新拖动。"); },
        async commit(snapshot) {
          await saves.synchronizeExternal?.(snapshot);
          if (!lease.commitGroup(snapshot, snapshot.page.revision, saves.synchronizeExternal ? () => undefined : install)) throw new Error("移动已保存，编辑器需要重新核对；当前输入已保留。");
        },
        cancel() { if (!lease.cancelBeforeDispatch() && !lease.rejectDispatched?.()) lease.requireRecovery(); }, recover() { lease.requireRecovery(); },
      };
    };
    const group = sessions.get(pageId) ?? new Set(); group.add(prepare); sessions.set(pageId, group);
    return () => { group.delete(prepare); if (!group.size) sessions.delete(pageId); };
  }, [pageId]);
}
