import { useEffect, useRef } from "react";
import { subscribeConversationCatalog } from "../../runtime/conversationCatalogEvents";

/** Coalesce mutation/focus signals; background work never polls or steals focus. */
export function useConversationCatalogSync(refresh: () => void): void {
  const latest = useRef(refresh); latest.current = refresh;
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const schedule = () => {
      if (document.visibilityState === "hidden" || timer !== undefined) return;
      timer = setTimeout(() => { timer = undefined; latest.current(); }, 0);
    };
    const unsubscribe = subscribeConversationCatalog(schedule);
    window.addEventListener("focus", schedule);
    document.addEventListener("visibilitychange", schedule);
    return () => {
      unsubscribe(); clearTimeout(timer);
      window.removeEventListener("focus", schedule);
      document.removeEventListener("visibilitychange", schedule);
    };
  }, []);
}
