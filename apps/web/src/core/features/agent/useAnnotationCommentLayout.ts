import {useLayoutEffect, useRef, useState, type RefObject} from "react";

/** Measure the compact width consistently, so wrapping cannot toggle the layout back and forth. */
export function useAnnotationCommentLayout(inputRef: RefObject<HTMLTextAreaElement | null>, creating: boolean, comment: string): boolean {
  const [multiline, setMultiline] = useState(false);
  const measured = useRef(false);
  useLayoutEffect(() => {
    const input = inputRef.current, root = input?.closest<HTMLElement>(".annotation-comment-editor");
    if (!input || !root) return;
    function fade() {
      if (!input) return;
      const scrollable = input.style.overflowY === "auto";
      input.toggleAttribute("data-fade-top", scrollable && input.scrollTop > 1);
      input.toggleAttribute("data-fade-bottom", scrollable && input.scrollHeight - input.clientHeight - input.scrollTop > 1);
    }
    function resize() {
      if (!input || !root) return;
      const width = root.clientWidth;
      if (!width) {if (!measured.current) setMultiline(comment.includes("\n")); return;}
      measured.current = true;
      const style = getComputedStyle(input), frame = getComputedStyle(root);
      const previousHeight = input.getBoundingClientRect().height;
      input.style.transition = "none";
      input.style.overflowY = "hidden";
      input.style.height = "0px";
      input.style.width = `${Math.max(1, width - parseFloat(frame.paddingLeft) - parseFloat(frame.paddingRight) - (creating ? 44 : 0))}px`;
      const oneLine = parseFloat(style.lineHeight) + parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      const nextMultiline = input.scrollHeight > Math.ceil(oneLine) + 1;
      input.style.width = "";
      const contentHeight = input.scrollHeight;
      const height = Math.min(140, Math.max(32, contentHeight));
      // During expansion the current animated height temporarily clips content.
      // Only the final height cap should enable scrolling, not that transient clip.
      input.style.overflowY = contentHeight > 140 ? "auto" : "hidden";
      input.style.height = `${previousHeight}px`;
      void input.offsetHeight;
      input.style.transition = "";
      input.style.height = `${height}px`;
      setMultiline(nextMultiline);
      fade();
    }
    let previousWidth = root.clientWidth;
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => {
      if (root.clientWidth !== previousWidth) {previousWidth = root.clientWidth; resize();}
      fade();
    });
    observer?.observe(root); observer?.observe(input);
    input.addEventListener("scroll", fade, {passive:true});
    input.addEventListener("transitionend", fade);
    resize();
    return () => {observer?.disconnect(); input.removeEventListener("scroll", fade); input.removeEventListener("transitionend", fade);};
  }, [inputRef, creating, comment, multiline]);
  return multiline;
}
