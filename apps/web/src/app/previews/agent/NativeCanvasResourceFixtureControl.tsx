import { useEffect, useRef, useState } from "react";
import type { NativeAgentPreview } from "../../../core/runtime/nativeAgentPreview";
import type { NativeCanvasFixture } from "./nativeCanvasScenarioTypes";
import { agentCanvasPreviewError } from "../../../core/features/agent/agentCanvasPreviewError";

/** Prepares only synthetic source data; production context controls own selection. */
export function NativeCanvasResourceFixtureControl({ preview, fixture, disabled, kind, onPrepared }: {
  readonly preview: NativeAgentPreview;
  readonly fixture: NativeCanvasFixture;
  readonly disabled: boolean;
  readonly kind: "paper" | "note";
  readonly onPrepared?: (id: string) => void;
}): React.JSX.Element {
  const label = kind === "paper" ? "论文" : "笔记";
  const [pending, setPending] = useState(false);
  const [title, setTitle] = useState<string>();
  const [error, setError] = useState<unknown>();
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  async function prepare(): Promise<void> {
    setPending(true); setError(undefined);
    try {
      const value = await preview.invoke(kind === "paper" ? "agent_preview_ensure_paper_fixture" : "agent_preview_ensure_note_fixture", {request: {
        fixtureId: fixture.fixtureId, generation: fixture.generation,
      }});
      if (!value || typeof value !== "object" || !("id" in value) || typeof value.id !== "string"
        || !("title" in value) || typeof value.title !== "string") throw new Error("AGENT_PREVIEW_RESOURCE_RESPONSE_INVALID");
      if (mounted.current) { setTitle(value.title); onPrepared?.(value.id); }
    } catch (cause) {
      if (mounted.current) setError(cause);
    } finally {
      if (mounted.current) setPending(false);
    }
  }
  const available = preview.bootstrap.supported.includes(`${kind}-resource-fixture`);
  return <>
    <button className="agent-canvas-demo-reset" type="button" disabled={disabled || pending || !available} onClick={() => void prepare()}>
      {pending ? `正在准备合成${label}…` : `准备合成${label}资源`}
    </button>
    {!available && <span>{label}夹具需要更新原生预览后端。</span>}
    {title && <span role="status">已准备“{title}”；可在独立对话的上下文中搜索并选择。</span>}
    {error != null && <span role="alert">{label}夹具准备失败：{agentCanvasPreviewError(error)}</span>}
  </>;
}
