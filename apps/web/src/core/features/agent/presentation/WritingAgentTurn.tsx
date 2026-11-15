import { ChevronRight } from "lucide-react";
import { Fragment, useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ShimmerText } from "../../../public/ui";
import { captureTurnDisclosureAnchor } from "./turnDisclosureAnchor";
import { formatAgentDuration } from "./formatAgentDuration";
import "./WritingAgentTurn.css";

export type WritingAgentTurnPart = {readonly kind: "process" | "final"; readonly key: string; readonly content: ReactNode};
interface WritingAgentTurnProps {
  readonly initialExpanded?: boolean | undefined;
  readonly onExpandedChange?: ((open: boolean) => void) | undefined;
  readonly children?: ReactNode;
  readonly finalMessage: ReactNode;
  readonly footer?: ReactNode;
  /** Ordered process segments and replies share one disclosure state. */
  readonly timeline?: readonly WritingAgentTurnPart[];
  readonly running: boolean;
  readonly aborted: boolean;
  readonly failed?: boolean;
  readonly hasProcess: boolean;
  readonly thinking: boolean;
  readonly reconnecting?:boolean;
  readonly startedAtMs?: number | undefined;
  readonly finishedAtMs?: number | undefined;
  readonly searchMessageId?: string | undefined;
  readonly turnId: string;
}

/** One disclosure owns the process, while the final reply stays outside it. */
export function WritingAgentTurn({ children, finalMessage, footer, timeline, running, aborted, failed = false, hasProcess, thinking, startedAtMs, finishedAtMs,
  searchMessageId, turnId, reconnecting=false, initialExpanded, onExpandedChange }: WritingAgentTurnProps): React.JSX.Element {
  const [open, setOpen] = useState(initialExpanded ?? running);
  const expandedChange = useRef(onExpandedChange); expandedChange.current = onExpandedChange;
  useEffect(() => {expandedChange.current?.(open);}, [open]);
  const wasRunning = useRef(running);
  const process = useRef<HTMLDivElement>(null);
  const section = useRef<HTMLElement>(null);
  const contentId = useId();
  const animateExpansion = useRef(false);
  const scrollAnchor = useRef<ReturnType<typeof captureTurnDisclosureAnchor>>(undefined);
  const start = useRef(startedAtMs ?? Date.now());
  const [elapsed, setElapsed] = useState(() => finishedAtMs === undefined ? 0
    : Math.max(0, Math.floor((finishedAtMs - start.current) / 1000)));

  useLayoutEffect(() => {
    scrollAnchor.current?.restore();
    const animate = animateExpansion.current;
    animateExpansion.current = false;
    if (!open || running || !animate
      || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const animations = [...(section.current?.querySelectorAll<HTMLElement>(".writing-agent-turn-content") ?? [])]
      .filter(content => !content.hidden && typeof content.animate === "function")
      .map(content => content.animate([
        { height: "0px", opacity: 0, overflow: "hidden" },
        { height: `${content.scrollHeight}px`, opacity: 1, overflow: "hidden" },
      ], { duration: 160, easing: "ease-out" }));
    return () => {for (const animation of animations) animation.cancel();};
  }, [open, running]);
  useEffect(() => () => scrollAnchor.current?.release(), []);

  useLayoutEffect(() => {
    if (wasRunning.current !== running) {
      setOpen(running);
      wasRunning.current = running;
    }
  }, [running]);

  useLayoutEffect(() => {
    if (searchMessageId && [...(section.current?.querySelectorAll<HTMLElement>(".writing-agent-turn-content [data-agent-message-id]") ?? [])]
      .some((item) => item.dataset.agentMessageId === searchMessageId)) {
      // Reveal before the parent's search effect measures and scrolls the hit.
      const disclosure = process.current?.closest("details");
      if (disclosure) disclosure.open = true;
      setOpen(true);
    }
  }, [searchMessageId]);

  useEffect(() => {
    if (startedAtMs !== undefined) start.current = startedAtMs;
    const update = () => setElapsed(Math.max(0, Math.floor(((finishedAtMs ?? Date.now()) - start.current) / 1000)));
    // Historical records without timestamps cannot invent a duration.
    if (running || finishedAtMs !== undefined || wasRunning.current) update();
    if (!running) return;
    update();
    const timer = window.setInterval(update, 1000);
    return () => { window.clearInterval(timer); update(); };
  }, [running, startedAtMs, finishedAtMs]);

  const label = `${running ? "工作中" : aborted ? "已中止" : failed ? "未完成" : "已完成"} · ${formatAgentDuration(elapsed)}`;
  const status = <ShimmerText active={false}>{label}</ShimmerText>;
  const thinkingStatus = running && thinking && !reconnecting && !finalMessage
    ? <div className="writing-agent-turn-header writing-agent-live-activity" data-agent-activity="thinking" role="status">
      <ShimmerText>思考中</ShimmerText>
    </div> : null;
  const first = timeline?.[0]?.kind === "process" ? timeline[0] : undefined;
  const rest = first ? timeline?.slice(1) : timeline;
  const lastProcess = timeline?.map(part => part.kind).lastIndexOf("process") ?? -1;
  const initialContent = timeline ? first?.content : children;
  const hasInitialContent = initialContent !== null && initialContent !== undefined;
  const initialThinking = !timeline || lastProcess < 0 || first && lastProcess === 0 ? thinkingStatus : null;
  const controlled = rest?.flatMap((part, index) => part.kind === "process" ? [`${contentId}-${index}`] : []).join(" ");
  return <section ref={section} className="writing-agent-turn" data-agent-turn-id={turnId}>
    {aborted && <div className="writing-agent-turn-header writing-agent-turn-aborted" role="status" data-agent-activity="aborted">{status}</div>}
    {!aborted && !hasProcess && !running && failed && <div className="writing-agent-turn-header" role="status" data-agent-activity="failed">
      <span className="writing-agent-turn-status">{status}</span>
    </div>}
    {(hasInitialContent || hasProcess || aborted) ? <details className="writing-agent-turn-process" data-flat={!hasProcess || aborted || undefined} open={!hasProcess || aborted || running || open}
      onToggle={(event) => {
        if (!hasProcess || aborted || running) { event.currentTarget.open = true; return; }
        setOpen(event.currentTarget.open);
      }}>
      <summary hidden={!hasProcess || aborted} className="writing-agent-turn-header" aria-label={running ? label : `${open ? "收起" : "展开"}${label}`}
        {...(controlled ? {"aria-controls": controlled} : {})}
        aria-disabled={running || undefined} tabIndex={running ? -1 : undefined}
        onClick={(event) => {
          event.preventDefault();
          if (running) return;
          scrollAnchor.current = captureTurnDisclosureAnchor(event.currentTarget);
          animateExpansion.current = !open;
          setOpen(!open);
        }}
        onKeyDown={(event) => { if (running && (event.key === "Enter" || event.key === " ")) event.preventDefault(); }}>
        <span className="writing-agent-turn-status" {...(running ? { role: "status", "data-agent-activity": "working" } : {})}>{status}</span>
        {!running && <ChevronRight aria-hidden="true" className="writing-agent-turn-chevron" size={14} />}
      </summary>
      <div className="writing-agent-turn-content" ref={process}>{initialContent}{initialThinking}</div>
    </details> : initialThinking}
    {timeline ? rest?.map((part, index) => part.kind === "final" ? <Fragment key={part.key}>{part.content}</Fragment>
      : <div key={part.key} id={`${contentId}-${index}`} className="writing-agent-turn-content" hidden={hasProcess && !running && !open && !aborted}>
        {part.content}{(index + (first ? 1 : 0)) === lastProcess ? thinkingStatus : null}
      </div>) : finalMessage}
    {footer}
  </section>;
}
