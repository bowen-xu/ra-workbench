import { useState } from "react";
import { FlaskConical } from "lucide-react";
import { useAppRuntime } from "../runtime/AppRuntimeProvider";
import type { NotePreviewScenario } from "../runtime/notePreviewScenarios";
import { noteReviewErrorMessage } from "../features/notes/editor/noteReviewRecovery";
import { notify } from "../services/notifications";
import { OverlayPortal } from "./overlay/OverlayPortal";
import { ActionMenu, ActionMenuItem } from "./ActionMenu";
import "./NotesPreviewControl.css";
const items: readonly [NotePreviewScenario, string][] = [
  ["open", "打开测试笔记"], ["conflict", "触发两处正文冲突"], ["review-conflict", "触发旧 AI 审阅冲突"],
  ["stale-proposal", "模拟正文已确认、旧提案过期"],
  ["ai-write", "模拟 AI 写入锁（3 秒）"], ["save-failure", "下次自动保存失败"],
  ["resolution-failure", "下次冲突消解失败（未写入）"], ["resolution-unknown", "下次消解回执丢失（已写入）"],
  ["reopen", "暂存并重新打开测试笔记"],
];
export function NotesPreviewControl(): React.JSX.Element | null {
  const { notePreview } = useAppRuntime();
  const [busy, setBusy] = useState(false);
  if (!notePreview) return null;
  async function run(scenario: NotePreviewScenario) {
    if (busy || !notePreview) return;
    setBusy(true);
    try {
      await notePreview.run(scenario);
      if (scenario === "save-failure") notify.info("已设置下次保存失败。请编辑正文并等待自动保存，再从保存状态旁重试。");
      else if (scenario.startsWith("resolution-")) notify.info("已设置消解故障。请打开冲突弹窗选择策略并确认保存。");
      else if (scenario.includes("conflict")) notify.info("已触发版本冲突。请点击工具栏最左侧的警告图标进行局部消解，也可暂存并重新打开。");
      else if (scenario === "stale-proposal") notify.info("正文已确认，旧提案仍过期。请打开右侧感叹号的恢复与诊断菜单，选择“保留当前内容并解决冲突”。");
    } catch (reason) { notify.error(noteReviewErrorMessage(reason)); }
    finally { setBusy(false); }
  }
  return <OverlayPortal><div className="notes-preview-control"><ActionMenu className="notes-preview-menu" portal label="笔记测试场景" trigger={<><FlaskConical size={14} />{busy ? "准备场景…" : "笔记测试场景"}</>} triggerClassName="notes-preview-trigger">
    {items.map(([scenario, label]) => <ActionMenuItem disabled={busy} key={scenario} onClick={() => void run(scenario)}>{label}</ActionMenuItem>)}
  </ActionMenu></div></OverlayPortal>;
}
