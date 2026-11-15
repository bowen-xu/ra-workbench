import { useLayoutEffect, useRef, type RefObject } from "react";
import { StreamRevealTimeline } from "./StreamRevealTimeline";

import "./StreamReveal.css";

/** Paint only pending glyphs, without reparsing Markdown on animation frames. */
export function useStreamReveal(host: RefObject<HTMLDivElement | null>, value: string, streaming: boolean, durationMs: number): boolean {
  const timeline = useRef<StreamRevealTimeline | undefined>(undefined);
  const hasStreamed = useRef(false);
  const frame = useRef<number | undefined>(undefined);
  const pending = useRef<HTMLElement[]>([]);
  const restoration = useRef<Animation | undefined>(undefined);
  const previousValue = useRef<string | undefined>(undefined);
  useLayoutEffect(() => {
    const element = host.current;
    const win = element?.ownerDocument.defaultView;
    if (!element || !win) return;
    if (streaming) hasStreamed.current = true;
    const now = win.performance.now();
    // Received text on mount is already available, including history loaded
    // when switching hosts. Only subsequent appends reveal new glyph ranges.
    if (!timeline.current) timeline.current = new StreamRevealTimeline(value, false, now, durationMs);
    else timeline.current.update(value, streaming, now, durationMs);
    if (frame.current !== undefined) win.cancelAnimationFrame(frame.current);
    const motion = win.matchMedia("(prefers-reduced-motion: reduce)");
    const paint = () => {
      const state = timeline.current!;
      if (motion.matches) state.settle();
      const cursor = state.position(win.performance.now());
      pending.current = pending.current.filter(node => {
        const start = Number(node.dataset.revealStart), end = Number(node.dataset.revealEnd);
        const progress = Math.min(1, Math.max(0, (cursor - start) / (end - start)));
        node.style.setProperty("--stream-reveal-progress", String(progress));
        return progress < 1;
      });
      element.dataset.streamRevealing = cursor < state.end ? "true" : "false";
      frame.current = cursor < state.end ? win.requestAnimationFrame(paint) : undefined;
    };
    pending.current = [...element.querySelectorAll<HTMLElement>(".stream-reveal-glyph")];
    paint();
    const restore = (replacement = false) => {
      if (frame.current !== undefined) win.cancelAnimationFrame(frame.current);
      frame.current = undefined;
      timeline.current?.settle();
      pending.current = [...element.querySelectorAll<HTMLElement>(".stream-reveal-glyph")];
      paint();
      restoration.current?.cancel();
      if (replacement && hasStreamed.current && value && !motion.matches && element.animate) {
        restoration.current = element.animate([{opacity:0},{opacity:1}], {duration:300,easing:"ease-out"});
      }
    };
    if (previousValue.current !== undefined && !value.startsWith(previousValue.current)) restore(true);
    previousValue.current = value;
    const onVisibility = () => { if (element.ownerDocument.visibilityState === "visible") restore(); };
    const onFocus = () => restore();
    win.addEventListener("focus", onFocus);
    element.ownerDocument.addEventListener("visibilitychange", onVisibility);
    const onMotionChange = () => {
      if (frame.current !== undefined) win.cancelAnimationFrame(frame.current);
      paint();
    };
    motion.addEventListener("change", onMotionChange);
    return () => {
      if (frame.current !== undefined) win.cancelAnimationFrame(frame.current);
      frame.current = undefined;
      motion.removeEventListener("change", onMotionChange);
      win.removeEventListener("focus", onFocus);
      element.ownerDocument.removeEventListener("visibilitychange", onVisibility);
    };
  }, [host, value, streaming, durationMs]);
  useLayoutEffect(() => {
    return () => restoration.current?.cancel();
  }, [host]);
  return streaming || hasStreamed.current;
}
