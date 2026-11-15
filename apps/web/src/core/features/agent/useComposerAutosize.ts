import {useLayoutEffect, useRef, type RefObject} from "react";
import type {AgentComposerInputHandle} from "./AgentComposerTextInput";

/** Measure outside normal flow: collapsing the live textarea clamps ancestor
 * scroll positions, even when its height is restored before the next paint. */
function measureTextareaHeight(input: HTMLTextAreaElement): number {
  const style = getComputedStyle(input);
  const mirror = input.cloneNode(false) as HTMLTextAreaElement;
  mirror.removeAttribute("id"); mirror.removeAttribute("name"); mirror.removeAttribute("aria-label");
  mirror.removeAttribute("placeholder"); mirror.removeAttribute("autofocus");
  mirror.setAttribute("aria-hidden", "true"); mirror.tabIndex = -1; mirror.inert = true;
  for (const property of ["width", "box-sizing", "font-family", "font-size", "font-weight", "font-style",
    "line-height", "letter-spacing", "word-spacing", "text-indent", "text-transform", "tab-size",
    "padding-top", "padding-right", "padding-bottom", "padding-left", "border-top-width", "border-bottom-width"]) {
    mirror.style.setProperty(property, style.getPropertyValue(property));
  }
  Object.assign(mirror.style, {position: "absolute", visibility: "hidden", pointerEvents: "none",
    top: "0", left: "0", height: "0", minHeight: "0", maxHeight: "none", overflow: "hidden",
    margin: "0", transition: "none", animation: "none"});
  mirror.value = input.value || " ";
  const pixels = (value: string) => Number.parseFloat(value) || 0;
  input.parentElement!.append(mirror);
  try {
    return mirror.scrollHeight + (style.boxSizing === "border-box"
      ? pixels(style.borderTopWidth) + pixels(style.borderBottomWidth)
      : -pixels(style.paddingTop) - pixels(style.paddingBottom));
  } finally { mirror.remove(); }
}

/** Reserve the reference row and controls before allocating the editor's space. */
export function useComposerAutosize(inputRef: RefObject<AgentComposerInputHandle | null>, rich: boolean, text: string, evidenceCount: number): void {
  const resizeRef = useRef<(() => void) | null>(null);
  useLayoutEffect(() => {
    const handle = inputRef.current;
    const editor = handle?.element ?? (handle instanceof HTMLTextAreaElement ? handle : null);
    const form = editor?.closest<HTMLElement>(".agent-reference-anchor");
    const panel = form?.closest<HTMLElement>(".writing-agent-panel, .note-agent-panel");
    const scroll = rich ? editor?.querySelector<HTMLElement>(".cm-scroller") : editor;
    if (!form || !panel || !scroll) return;
    let frame = 0;
    const updateFade = () => scroll.toggleAttribute("data-scrolled-top", scroll.scrollTop > 1);
    function resize() {
      frame = 0;
      if (!form || !panel || !scroll) return;
      const panelHeight = panel.getBoundingClientRect().height;
      if (!panelHeight) return;
      const inputBox = rich ? editor?.parentElement : scroll;
      if (!inputBox) return;
      const reserved = form.getBoundingClientRect().height - inputBox.getBoundingClientRect().height;
      const maxHeight = Math.max(1, Math.floor(panelHeight * .35 - reserved));
      const maximum = `${maxHeight}px`;
      if (form.style.getPropertyValue("--agent-composer-input-max-height") !== maximum) {
        form.style.setProperty("--agent-composer-input-max-height", maximum);
      }
      if (scroll instanceof HTMLTextAreaElement) {
        const previousScroll = scroll.scrollTop;
        const contentHeight = measureTextareaHeight(scroll);
        const height = `${Math.min(contentHeight, maxHeight)}px`;
        const overflow = contentHeight > maxHeight ? "auto" : "hidden";
        if (scroll.style.height !== height) scroll.style.height = height;
        if (scroll.style.overflowY !== overflow) scroll.style.overflowY = overflow;
        if (scroll.scrollTop !== previousScroll) scroll.scrollTop = previousScroll;
      }
      updateFade();
    }
    const schedule = () => {if (!frame) frame = requestAnimationFrame(resize);};
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    observer?.observe(panel); observer?.observe(form);
    if (rich) {const content = editor?.querySelector(".cm-content"); if (content) observer?.observe(content);}
    scroll.addEventListener("scroll", updateFade, {passive: true});
    window.addEventListener("resize", schedule);
    resizeRef.current = resize;
    return () => {resizeRef.current = null; observer?.disconnect(); cancelAnimationFrame(frame); scroll.removeEventListener("scroll", updateFade); window.removeEventListener("resize", schedule);};
  }, [inputRef, rich]);
  useLayoutEffect(() => {resizeRef.current?.();}, [rich, text, evidenceCount]);
}
