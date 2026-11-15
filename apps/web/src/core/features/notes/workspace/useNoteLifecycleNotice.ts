import type { NotePage } from "@research-assistant/core/notes";
import { useEffect, useRef } from "react";
import { notify } from "../../../services/notifications";

/** Archive feedback only; viewing trash remains silent. */
export function useNoteLifecycleNotice(page: NotePage | undefined, visible: boolean): void {
  const previous = useRef<string | undefined>(undefined);
  const id = page?.id;
  const status = page?.status;
  useEffect(() => {
    const key = visible && id && status === "archived" ? `${id}:${status}` : undefined;
    if (key && previous.current !== key) {
      notify.info("笔记已归档，当前内容只读。", { id: `note-lifecycle:${id}` });
    }
    previous.current = key;
  }, [id, status, visible]);
}
