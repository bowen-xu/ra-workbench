import {useEffect, useLayoutEffect, useRef, useState} from "react";
import {useNestedWheelScroll} from "../../../public/layout";
import type { AgentMessageSearchTarget } from "../../../public/agentPresentation/useMessageSearchTarget";
import {releaseTurnDisclosureAnchor} from "./turnDisclosureAnchor";
import "./TranscriptScrollbar.css";

/** Own viewport follow, user intent and virtual-height anchoring independently
 * of the composer and the durable conversation state. */
export function useAgentTranscriptScroll({open, historyError, historyReady, searchTarget, messages, childTasks}: {
  readonly open: boolean; readonly historyError: boolean; readonly historyReady: boolean;
  readonly searchTarget: AgentMessageSearchTarget | null | undefined;
  readonly messages: readonly unknown[]; readonly childTasks: readonly unknown[];
}) {
  const messagesRef = useRef<HTMLDivElement>(null);
  const transcriptContentRef = useRef<HTMLDivElement>(null);
  const stickMessagesToBottomRef = useRef(true);
  const followMessagesRef = useRef<(() => void) | null>(null);
  const searchAnchorRef = useRef<typeof searchTarget>(undefined);
  const [awayFromBottom, setAwayFromBottom] = useState(false);
  useNestedWheelScroll(messagesRef,open,(owner,deltaY)=>{
    if(owner!==messagesRef.current || deltaY < 0){
      if (messagesRef.current) {
        delete messagesRef.current.dataset.transcriptAnchoring;
        messagesRef.current.dataset.followBottom = "false";
      }
      stickMessagesToBottomRef.current=false;
      setAwayFromBottom(true);
    }
  });
  useEffect(() => {
    const messagesElement = messagesRef.current;
    if (!open || !messagesElement) return;
    const updateStickiness = (): void => {
      const distanceFromBottom = messagesElement.scrollHeight - messagesElement.clientHeight - messagesElement.scrollTop;
      const atBottom = messagesElement.scrollHeight === 0 ? messagesElement.scrollTop <= 0 : distanceFromBottom <= 6;
      // Replacing height estimates can clamp scrollTop before the window has
      // restored its anchor. This is layout work, not a user leaving the end.
      if (!atBottom && stickMessagesToBottomRef.current && messagesElement.dataset.transcriptAnchoring === "true") return;
      stickMessagesToBottomRef.current = atBottom;
      setAwayFromBottom(!atBottom);
    };
    const preserveProgrammaticPosition = (): void => {
      stickMessagesToBottomRef.current = false;
      setAwayFromBottom(true);
    };
    const manualScroll = (): void => {
      delete messagesElement.dataset.transcriptAnchoring;
      messagesElement.dataset.followBottom = "false";
      preserveProgrammaticPosition();
    };
    const scrollKey = (event: globalThis.KeyboardEvent): void => {
      if (["PageUp", "PageDown", "Home", "End", "ArrowUp", "ArrowDown"].includes(event.key)) manualScroll();
    };
    messagesElement.addEventListener("scroll", updateStickiness, { passive: true });
    messagesElement.addEventListener("keydown", scrollKey, true);
    messagesElement.addEventListener("touchmove", manualScroll, {passive:true, capture:true});
    messagesElement.addEventListener("writing-agent-preserve-position", preserveProgrammaticPosition);
    updateStickiness();
    return () => {
      messagesElement.removeEventListener("scroll", updateStickiness);
      messagesElement.removeEventListener("keydown", scrollKey, true);
      messagesElement.removeEventListener("touchmove", manualScroll, true);
      messagesElement.removeEventListener("writing-agent-preserve-position", preserveProgrammaticPosition);
    };
  }, [open]);

  useLayoutEffect(() => {
    const messagesElement = messagesRef.current;
    const transcriptContent = transcriptContentRef.current;
    if (open && searchTarget && searchAnchorRef.current !== searchTarget) {
      searchAnchorRef.current = searchTarget;
      stickMessagesToBottomRef.current = false;
    }
    if (!open || !messagesElement || !stickMessagesToBottomRef.current) return;
    let frame: number | undefined;
    const scrollToBottom = (): void => {
      if (stickMessagesToBottomRef.current && messagesElement.scrollHeight > messagesElement.clientHeight) {
        messagesElement.scrollTop = Math.max(0, messagesElement.scrollHeight - messagesElement.clientHeight);
      }
    };
    followMessagesRef.current = scrollToBottom;
    scrollToBottom();
    const scheduleScroll = (): void => {if (frame === undefined) frame = window.requestAnimationFrame(() => {frame = undefined; scrollToBottom();});};
    scheduleScroll();
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(scheduleScroll);
    observer?.observe(messagesElement);
    if (transcriptContent) observer?.observe(transcriptContent);
    const mutationObserver = typeof MutationObserver === "undefined" ? undefined : new MutationObserver(scheduleScroll);
    mutationObserver?.observe(transcriptContent ?? messagesElement, { childList: true, characterData: true, subtree: true });
    return () => {
      followMessagesRef.current = null;
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      observer?.disconnect();
      mutationObserver?.disconnect();
    };
  }, [historyError, historyReady, searchTarget, open]);
  useLayoutEffect(() => {followMessagesRef.current?.();}, [messages, childTasks]);

  function returnToBottom(): void {
    const element = messagesRef.current;
    if (!element) return;
    releaseTurnDisclosureAnchor(element);
    stickMessagesToBottomRef.current = true;
    element.scrollTop = Math.max(0, element.scrollHeight - element.clientHeight);
    setAwayFromBottom(false);
  }
  return {messagesRef, transcriptContentRef, awayFromBottom, returnToBottom};
}
