import type { ReactNode } from "react";
import { AppToaster } from "../../../core/components/AppToaster";
import { NotificationRestoreButton } from "../../../core/components/NotificationRestoreButton";
import "./AgentCanvasDemo.css";

/** Shared outer harness. Its children remain the production workspace tree. */
export function AgentCanvasPreviewFrame({ children, tabs, documents, controls, workspaceBlocked = false }: {
  readonly children: ReactNode; readonly tabs?: ReactNode; readonly documents?: ReactNode; readonly controls: ReactNode;
  readonly workspaceBlocked?: boolean;
}): React.JSX.Element {
  return <main className="agent-canvas-demo"><AppToaster />
    <header className="agent-canvas-demo-header"><div>
      <span className="agent-canvas-demo-eyebrow">AGENT CANVAS / UI PLAYGROUND</span>
      <h1>队列消息 UI 预览</h1><p>实际聊天组件和运行控制器；以假数据模拟技能、工具、排队和 Steer。</p>
    </div><NotificationRestoreButton /></header>
    <section className="agent-canvas-demo-workspace" inert={workspaceBlocked}>{tabs}<div className="agent-canvas-demo-surface">{children}</div></section>
    {documents}
    <footer className="agent-canvas-demo-footer">{controls}</footer>
  </main>;
}
