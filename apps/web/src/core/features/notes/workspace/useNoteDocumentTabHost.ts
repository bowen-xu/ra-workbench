import { useEffect, useRef } from "react";
import type { WorkspaceView } from "../../../useWorkspaceNavigation";
import type { WorkspacePresentation } from "../../../components/workspace-tabs/WorkspacePresentation";
import { navigateSource } from "../../agent/sourceNavigation";
import { OPEN_NOTE_DOCUMENT_TAB, type OpenNoteDocumentTabRequest } from "./openNoteDocumentTab";

export function useNoteDocumentTabHost(options: {
  readonly chooseView: (view: WorkspaceView, destination?: string, ready?: () => void,
    presentation?: WorkspacePresentation, cancelled?: () => void) => boolean;
  readonly openPage: (id: string) => void;
}) {
  const current = useRef(options); current.current = options;
  useEffect(() => {
    const pending = new Set<OpenNoteDocumentTabRequest>();
    const fail = (request: OpenNoteDocumentTabRequest, reason: unknown) => { if (pending.delete(request)) request.reject(reason); };
    const acknowledge = (request: OpenNoteDocumentTabRequest) => { if (pending.delete(request)) request.onOpened(); };
    const open = (event: Event) => {
      const request = (event as CustomEvent<OpenNoteDocumentTabRequest>).detail;
      event.preventDefault(); pending.add(request);
      if (request.target.kind === "writing") {
        const path = request.target.path.split("/").map(encodeURIComponent).join("/");
        void navigateSource(`source://writings/${encodeURIComponent(request.target.manuscriptId)}/${path}`, {presentation: "document"})
          .then(() => acknowledge(request), reason => fail(request, reason));
        return;
      }
      const id = request.target.pageId;
      const cancelled = () => fail(request, new Error("无法打开笔记标签页，请先保存或解决当前编辑状态。"));
      if (!current.current.chooseView("notes", id, () => {
        if (!pending.has(request)) return;
        current.current.openPage(id);
        // Release the auxiliary owner before the retained document host mounts/loads.
        acknowledge(request);
      }, "document", cancelled)) cancelled();
    };
    window.addEventListener(OPEN_NOTE_DOCUMENT_TAB, open);
    return () => {
      window.removeEventListener(OPEN_NOTE_DOCUMENT_TAB, open);
      for (const request of pending) fail(request, new Error("笔记标签页宿主已关闭"));
    };
  }, []);
}
