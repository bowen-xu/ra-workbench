import { useCallback, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { notify } from "./notifications";

/** Retain a recoverable state without rendering a private message banner. */
export function useNotificationState(kind: "error" | "info" = "error"): [string | undefined, Dispatch<SetStateAction<string | undefined>>] {
  const [message, setMessage] = useState<string>();
  const latest = useRef(message);
  const update = useCallback<Dispatch<SetStateAction<string | undefined>>>((action) => {
    const next = typeof action === "function" ? action(latest.current) : action;
    latest.current = next;
    setMessage(next);
    // A second failed attempt should notify even if its message is identical.
    if (next) notify[kind](next);
  }, [kind]);
  return [message, update];
}
