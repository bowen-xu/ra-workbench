import { useState } from "react";
import type { AgentQueueEdit, AgentQueueState } from "@research-assistant/core/agent";
import type { PortraitTemplateField } from "@research-assistant/core/ai";
import { AppToaster } from "../../../core/components/AppToaster";
import { WorkspaceTabBar, WorkspaceTabsProvider, usePublishWorkspaceTabs } from "../../../core/components/workspace-tabs/WorkspaceTabs";
import { AgentInputQueue } from "../../../core/features/agent/AgentInputQueue";
import { ReadingTemplateFields } from "../../../../../../plugins/literature_library/src/features/ai/ReadingTemplateFields";
import { notify } from "../../../core/services/notifications";
import "../../../../../../plugins/literature_library/src/features/ai/PaperPortraitPanel.css";
import "../../../../../../docs/ui-templates/reorder-template/Example.css";

const initialFields: readonly PortraitTemplateField[] = [
  { id: "question", title: "研究问题", question: "研究希望回答的问题。", claimIds: [], color: "#4b8e62" },
  { id: "method", title: "方法", question: "采用的方法、实验条件和关键假设。", claimIds: [], color: "#4b82b2" },
  { id: "findings", title: "结果与贡献", question: "证据支持的主要结果。", claimIds: [], color: "#b48738" },
];
const initialQueue: AgentQueueState = { schemaVersion: 1, conversationId: "reorder-example", revision: 0, paused: true, preparingInputId: null,
  inputs: [{ inputId: "question", text: "先梳理研究问题" }, { inputId: "method", text: "核对方法与实验条件" }, { inputId: "findings", text: "整理主要结果及证据" }] };

function TabsExample(): React.JSX.Element {
  usePublishWorkspaceTabs("example", [
    { id: "home", home: true, owner: "library", title: "返回文献列表", active: false, select() {} },
    ...["研究问题.pdf", "方法与实验.pdf", "结果与贡献.pdf"].map((title, index) => ({ id: `example:${index}`, title, owner: "library" as const, icon: "pdf" as const, active: index === 0, select() {} })),
  ]);
  return <WorkspaceTabBar owner="library" textActive={false} navigate={(_, __, ready) => { ready(); return true; }} />;
}

/** Only data and save callbacks are mocked; all three hosts use production UI. */
export function ReorderExample(): React.JSX.Element {
  const [fields, setFields] = useState(initialFields);
  const [queue, setQueue] = useState(initialQueue);
  const [saving, setSaving] = useState(false);
  const [mode, setMode] = useState("immediate");
  async function editQueue(edit: AgentQueueEdit): Promise<boolean> {
    setSaving(true);
    if (mode !== "immediate") await new Promise<void>(resolve => window.setTimeout(resolve, 1200));
    if (mode === "failure") { setSaving(false); notify.error("模拟保存失败，已恢复原顺序。"); return false; }
    setQueue(current => ({ ...current, revision: current.revision + 1, inputs: edit.type === "reorder"
      ? edit.inputIds.flatMap(id => current.inputs.filter(input => input.inputId === id))
      : edit.type === "remove" ? current.inputs.filter(input => input.inputId !== edit.inputId) : current.inputs }));
    setSaving(false); return true;
  }
  return <main className="reorder-example">
    <AppToaster />
    <header><h1>拖动排序参考</h1><p>跨过相邻边界换位 · 双阈值防抖 · 平滑让位 · 松手无跳位 · Esc 取消</p></header>
    <section aria-label="横向标签示例"><h2>横向 · 窗口标签</h2><p>拖动文件标签，首页固定。也可使用 Alt＋左右方向键排序。</p>
      <div className="reorder-example-tab-frame"><WorkspaceTabsProvider><TabsExample /></WorkspaceTabsProvider></div>
    </section>
    <section aria-label="纵向消息队列示例"><h2>纵向 · 消息队列</h2>
      <label>保存场景 <select aria-label="保存场景" value={mode} disabled={saving} onChange={event => setMode(event.target.value)}>
        <option value="immediate">立即保存</option><option value="delayed">延迟 1.2 秒保存</option><option value="failure">延迟后保存失败</option>
      </select></label><p>等待保存期间保留新顺序；失败时平滑回退，消息内容不丢失。</p>
      <AgentInputQueue state={{ queue, editing: saving, loading: false, uncertain: false, error: null }} disabled={false}
        onEdit={editQueue} onReload={() => undefined} onRetry={() => undefined} />
    </section>
    <section aria-label="纵向阅读模板示例"><h2>纵向 · 阅读模板</h2><p>复用原有字段编辑、颜色、删除和键盘排序控件。</p>
      <div className="paper-portrait-editor"><ReadingTemplateFields fields={fields} disabled={false} onChange={setFields} /></div>
    </section>
  </main>;
}
