import { useEffect, useState, type ReactNode } from "react";
import type { AppRuntime } from "./appRuntime";
import { ensureStructuredResourceStartup } from "./structuredResourceStartup";
import "./StructuredResourceStartupGate.css";

/** Business components do not mount until legacy bodies, templates and grants are durable. */
export function StructuredResourceStartupGate({runtime, children}: {
  readonly runtime: AppRuntime;
  readonly children: ReactNode;
}) {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{ready: boolean; error?: string}>({ready: false});
  useEffect(() => {
    let active = true;
    setState({ready: false});
    void ensureStructuredResourceStartup(runtime).then(() => {
      if (active) setState({ready: true});
    }).catch(reason => {
      if (active) setState({ready: false, error: reason instanceof Error ? reason.message : "旧阅读内容迁移失败，原数据已保留。"});
    });
    return () => { active = false; };
  }, [runtime, attempt]);
  if (state.ready) return children;
  return (
    <main className="app-startup-state" aria-label="恢复阅读资源">
      {state.error ? <section role="alert">
        <h1>暂时无法完成数据恢复</h1>
        <p>{state.error}</p>
        <p>完成恢复后即可继续使用对话与阅读功能。</p>
        <button type="button" onClick={() => setAttempt(value => value + 1)}>重试恢复</button>
      </section> : <p role="status">正在恢复阅读内容、模板与对话上下文…</p>}
    </main>
  );
}
