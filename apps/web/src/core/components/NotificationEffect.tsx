import { useEffect, useRef } from "react";
import { notify } from "../services/notifications";

/** Project retained state through the application toaster without an inline banner.
 * Dismissing a notification never resolves the owner's pending operation.
 */
export function NotificationEffect({ id, message, kind = "warning" }: {
  readonly id: string;
  readonly message: string | undefined;
  readonly kind?: "warning" | "error" | "info";
}): null {
  const active = useRef<{ id: string; message: string | undefined } | null>(null);
  useEffect(() => {
    const owner = { id, message };
    active.current = owner;
    if (!message) return;
    notify[kind](message, { id });
    // Sonner defers dismissal to an animation frame. StrictMode's immediate
    // cleanup/setup must not schedule removal of the newly published same ID.
    return () => { queueMicrotask(() => {
      const latest = active.current;
      if (latest === owner || latest?.id !== id || !latest.message) notify.dismiss(id);
    }); };
  }, [id, message, kind]);
  return null;
}
