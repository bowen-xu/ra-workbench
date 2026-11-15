import { ControlHint } from "../../components/ControlHint";
import type { Components } from "react-markdown";
import { toJsxRuntime } from "hast-util-to-jsx-runtime";
import { jsx, jsxs, Fragment } from "react/jsx-runtime";
import { Check, Copy } from "lucide-react";
import { memo, startTransition, useEffect, useRef, useState, type ReactNode } from "react";
import { StreamReveal } from "../../components/StreamReveal/StreamReveal";
import { copyText } from "./copyText";
import "katex/dist/katex.min.css";
import { formatSourceLink, parseSourceLink } from "@research-assistant/core/source";
import { AgentSourceLink } from "./AgentSourceLink";
import { AgentMathViewport } from "./AgentMathViewport";
import { useMarkdownRenderQueue } from "./markdown/MarkdownRenderScope";
import type { MarkdownSnapshot } from "./markdown/MarkdownRenderQueue";
import type { MarkdownBlock } from "./markdown/markdownProtocol";
import { notify } from "../../services/notifications";
import {useMarkdownSelectionCopy} from "./useMarkdownSelectionCopy";

function textContent(value: ReactNode): string {
  if (typeof value === "string" || typeof value === "number") return String(value);
  if (Array.isArray(value)) return value.map(textContent).join("");
  if (value && typeof value === "object" && "props" in value) {
    return textContent((value as { props?: { children?: ReactNode } }).props?.children);
  }
  return "";
}

function CopyButton({ value, label = "复制" }: { readonly value: string; readonly label?: string }): React.JSX.Element {
  const [copied, setCopied] = useState(false);
  async function copy(): Promise<void> {
    if (!value) return;
    if (await copyText(value)) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    }
  }
  return <ControlHint content={copied ? "已复制" : label}><button aria-label={copied ? "已复制" : label} className="writing-agent-copy-button" onClick={() => void copy()}  type="button">
    {copied ? <Check aria-hidden="true" size={13} /> : <Copy aria-hidden="true" size={13} />}
  </button></ControlHint>;
}

// A dedicated AST tag avoids installing a React component for every KaTeX span.
const markdownComponents: Components & { "agent-math": NonNullable<Components["span"]> } = {
  "agent-math": ({ children, node }) => <AgentMathViewport display={node?.properties["data-math-display"] === true}>{children}</AgentMathViewport>,
  pre: ({ children, ...props }) => {
    const value = textContent(children);
    return <div className="writing-agent-code-block">
      <pre {...props}>{children}</pre>
      <CopyButton label="复制代码" value={value} />
    </div>;
  },
  a: ({ children, href, node: _node, ...props }) => {
    if (!href?.startsWith("source://")) return <a {...props} href={href} rel="noreferrer" target="_blank">{children}</a>;
    let path: string;
    try { path = formatSourceLink(parseSourceLink(href)); } catch { return <span aria-disabled="true">{children}</span>; }
    return <AgentSourceLink {...props} href={href} path={path}>{children}</AgentSourceLink>;
  },
};

export const AgentMarkdown = memo(function AgentMarkdown({
  streaming = false,
  value,
}: {
  readonly streaming?: boolean;
  readonly value: string;
}): React.JSX.Element {
  const element = useRef<HTMLDivElement>(null);
  useMarkdownSelectionCopy(element);
  const queue = useMarkdownRenderQueue();
  const [snapshot, setSnapshot] = useState<MarkdownSnapshot | null>(null);
  const connection = useRef<ReturnType<typeof queue.connect> | null>(null);
  const hasStreamed = useRef(streaming);
  if (streaming) hasStreamed.current = true;
  const latest = useRef({value, streaming, reveal: hasStreamed.current});
  latest.current = {value, streaming, reveal: hasStreamed.current};
  useEffect(() => {
    let alive = true, reported = false;
    const client = queue.connect(next => {
      if (!alive) return;
      if (next.failed && !reported) {
        reported = true;
        notify.error("AGENT_MARKDOWN_RENDER_FAILED：消息排版失败，已保留原始文本。");
      }
      // Worker completion is output-only, interruptible React work. Never put
      // composer updates in this transition or fall back to parsing on the UI thread.
      startTransition(() => setSnapshot(next));
    });
    connection.current = client; client.update(latest.current);
    return () => {alive = false; connection.current = null; client.dispose();};
  }, [queue]);
  useEffect(() => {connection.current?.update(latest.current);}, [value, streaming]);
  return <StreamReveal elementRef={element} value={snapshot?.value ?? ""} streaming={snapshot?.streaming ?? streaming} className="writing-agent-markdown">{() =>
    snapshot?.failed ? snapshot.value : snapshot?.blocks.map((block, index) => <CompiledMarkdownBlock key={index} block={block}/>)
  }</StreamReveal>;
});

/** Stable block identity skips both HAST -> React conversion and reconciliation
 * for the completed prefix; a later Markdown dependency may invalidate it. */
const CompiledMarkdownBlock = memo(function CompiledMarkdownBlock({block}: {readonly block: MarkdownBlock}): React.JSX.Element {
  return toJsxRuntime({type: "root", children: [block.tree]}, {
    Fragment, jsx, jsxs, components: markdownComponents, passKeys: true, passNode: true, ignoreInvalidStyle: true,
  });
});
