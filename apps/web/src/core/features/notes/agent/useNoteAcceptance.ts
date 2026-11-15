import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import type { AgentNoteApplication, AgentNoteRewriteReview } from "@research-assistant/core/agent";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import { agentNoteApplicationGateway, agentNoteUndoGateway } from "../../../services/agentGateway";
import { notify } from "../../../services/notifications";
import type { NoteEditorInstallPort } from "../editor/noteEditorInstallPort";
import type { NoteSaveCoordinator } from "../editor/noteSaveController";
import { NoteAcceptanceController, type NoteAcceptanceState } from "./noteAcceptanceController";

export function useNoteAcceptance(options: {
  readonly snapshot: NoteDocumentSnapshot | undefined;
  readonly editorSessionId: number;
  readonly saves: RefObject<NoteSaveCoordinator | undefined>;
  readonly notes: NotesGateway;
}) {
  const live = useRef(options); live.current = options;
  const editor = useRef<NoteEditorInstallPort | undefined>(undefined);
  const controller = useRef<NoteAcceptanceController | undefined>(undefined);
  const [state, setState] = useState<NoteAcceptanceState>({ phase: "idle" });
  useEffect(() => {
    const owned = new NoteAcceptanceController({ gateway: agentNoteApplicationGateway, undoGateway: agentNoteUndoGateway, notes: options.notes,
      onState: setState,
      onApplied: kind => notify.success(kind === "undo" ? "改写已撤销并保存，原应用记录保留。" : "改写已应用并保存，可通过编辑器撤销。"),
      current: () => {
        const { snapshot, saves } = live.current;
        return snapshot && saves.current && editor.current ? { snapshot, saves: saves.current, editor: editor.current } : undefined;
      },
    });
    controller.current = owned;
    return () => { owned.dispose(); controller.current = undefined; };
  }, [options.notes]);
  useEffect(() => { controller.current?.resetSession(); }, [options.snapshot?.page.id, options.editorSessionId]);
  const registerEditor = useCallback((port: NoteEditorInstallPort | undefined) => { editor.current = port; }, []);
  const accept = useCallback(async (review: AgentNoteRewriteReview): Promise<void> => {
    if (!controller.current) throw new Error("笔记编辑器尚未就绪。");
    try { await controller.current.accept(review); }
    catch (reason: unknown) {
      notify.error(reason instanceof Error ? reason.message : "接受结果尚未确认，请核对原操作。");
      throw reason;
    }
  }, []);
  const retry = useCallback(async (): Promise<void> => {
    try { await controller.current?.retry(); }
    catch { notify.error("尚未完成核对，请比较当前与已保存版本。"); }
  }, []);
  const undo = useCallback(async (review: AgentNoteRewriteReview, application: AgentNoteApplication): Promise<void> => {
    if (!controller.current) throw new Error("笔记编辑器尚未就绪。");
    try { await controller.current.undo(review, application); }
    catch (reason: unknown) {
      notify.error(reason instanceof Error ? reason.message : "撤销结果尚未确认，请核对原操作。");
      throw reason;
    }
  }, []);
  return { state, registerEditor, accept, undo, retry };
}
