import { createContext, useCallback, useContext, useRef, type ReactNode, type RefObject } from "react";
import { useStreamReveal } from "./useStreamReveal";
import { STREAM_REVEAL_DURATION_MS } from "./StreamRevealTimeline";

const durationContext = createContext(STREAM_REVEAL_DURATION_MS);

/** Scope presentation timing without threading it through business components. */
export function StreamRevealDurationProvider({ durationMs, children }: {
  readonly durationMs: number;
  readonly children: ReactNode;
}): React.JSX.Element {
  return <durationContext.Provider value={durationMs}>{children}</durationContext.Provider>;
}

/** Presentation-only reveal boundary. A renderer marks visible text ranges with
 * .stream-reveal-glyph and data-reveal-start/end source offsets. It needs no
 * knowledge of messages, providers, Markdown, or the response lifecycle. */
export function StreamReveal({ value, streaming = false, durationMs, className, children, elementRef }: {
  readonly value: string;
  readonly streaming?: boolean;
  readonly durationMs?: number;
  readonly className?: string;
  readonly children: (enabled: boolean) => ReactNode;
  readonly elementRef?: RefObject<HTMLDivElement | null>;
}): React.JSX.Element {
  const host = useRef<HTMLDivElement>(null);
  const setHost = useCallback((node: HTMLDivElement | null) => {host.current = node; if (elementRef) elementRef.current = node;}, [elementRef]);
  const defaultDuration = useContext(durationContext);
  const enabled = useStreamReveal(host, value, streaming, durationMs ?? defaultDuration);
  return <div ref={setHost} aria-busy={streaming} className={className}>{children(enabled)}</div>;
}
