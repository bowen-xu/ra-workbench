import { useEffect, useRef, useState, type RefObject } from "react";
import type { AgentCatalogGateway } from "@research-assistant/data/agent";
import type { AgentConversationActivity, AgentConversationBinding } from "@research-assistant/core/agent";
import { useConversationCatalogSync } from "./useConversationCatalogSync";

/** One small lifecycle query for the host, including off-page/internal chats.
 * No history readers, writer leases, selection changes or overlapping requests. */
export function useConversationActivity(catalog: AgentCatalogGateway, binding: AgentConversationBinding | null, element: RefObject<HTMLElement | null>) {
  const [running, setRunning] = useState<readonly AgentConversationActivity[]>([]);
  const refresh = useRef(() => {});
  useConversationCatalogSync(() => refresh.current());
  useEffect(() => {
    const read = catalog.activity;
    if (!read) return;
    let disposed = false, pending = false, requested = false;
    let unsubscribe: (() => void) | undefined;
    function start(): void {
      if (disposed) return;
      if (pending) { requested = true; return; }
      void readActivity();
    }
    async function readActivity(): Promise<void> {
      pending = true;
      try {
        if (document.visibilityState !== "hidden" && !element.current?.closest("[hidden]")) {
          const next = await read!({binding});
          if (!disposed) setRunning(next);
        }
      } catch { if (!disposed) setRunning([]); }
      finally {
        pending = false;
        const again = requested;
        requested = false;
        if (!disposed && again) start();
      }
    }
    refresh.current = start;
    // Register before the baseline, then bridge async native-listener setup once.
    if (catalog.subscribeActivity) void catalog.subscribeActivity(start).then(stop => {
      if (disposed) stop();
      else { unsubscribe = stop; start(); }
    }).catch(() => { if (!disposed) start(); }); // Focus/manual signals still permit recovery.
    // The drawer stays mounted while hidden; reopening must refresh skipped notices.
    const visibility = new MutationObserver(() => {
      if (!element.current?.closest("[hidden]")) start();
    });
    for (let node = element.current; node; node = node.parentElement)
      visibility.observe(node, {attributes: true, attributeFilter: ["hidden"]});
    start();
    return () => { disposed = true; refresh.current = () => {}; unsubscribe?.(); visibility.disconnect(); };
  }, [catalog, binding?.kind, binding?.resourceId, element]);
  return running;
}
