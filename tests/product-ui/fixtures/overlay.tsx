import { StrictMode, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { DialogHeading, Select, SelectItem, SelectProvider } from "@ariakit/react";
import { ActionMenu, ActionMenuItem } from "../../../apps/web/src/core/components/ActionMenu";
import { ControlHint } from "../../../apps/web/src/core/components/ControlHint";
import { Dialog } from "../../../apps/web/src/core/components/overlay/OverlayDialog";
import { SelectPopover } from "../../../apps/web/src/core/components/overlay/OverlayPopup";
import { StructuredReviewControls } from "../../../apps/web/src/core/public/structuredReview";
import { WritingCommandPalette } from "../../../../plugins/ra-writing-projects/src/features/writing/WritingCommandPalette";
import "../../../apps/web/src/core/styles.css";
import "../../../apps/web/src/core/public/structuredReview.css";
import "../../../../plugins/ra-writing-projects/src/features/writing/WritingWorkspace.css";
import "../../../../plugins/ra-writing-projects/src/features/writing/WritingWorkbench.css";
import "./overlay.css";
import { AppToaster } from "../../../apps/web/src/core/components/AppToaster";
import { notify } from "../../../apps/web/src/core/services/notifications";

function Controls({ name, onNested }: { name: string; onNested?: () => void }) {
  const [result, setResult] = useState("");
  return <>
    <ControlHint content={`${name}来源提示`}><button>{name}来源</button></ControlHint>
    <ActionMenu label={`${name}菜单`} className="fixture-menu" trigger="菜单" contentClassName="fixture-popup" hint>
      <ControlHint content={`${name}菜单项提示`}><ActionMenuItem onClick={() => setResult("完成")}>执行</ActionMenuItem></ControlHint>
      <ActionMenu label={`${name}次级菜单`} className="fixture-menu" trigger="次级" contentClassName="fixture-popup">
        <ActionMenuItem onClick={() => setResult("次级完成")}>执行次级</ActionMenuItem>
      </ActionMenu>
      {onNested && <ActionMenuItem onClick={onNested}>打开子弹窗</ActionMenuItem>}
    </ActionMenu>
    <SelectProvider defaultValue="甲"><Select aria-label={`${name}选择`} />
      <SelectPopover className="fixture-popup"><SelectItem value="甲" /><SelectItem value="乙" /></SelectPopover>
    </SelectProvider>
    <output aria-label={`${name}结果`}>{result}</output>
  </>;
}

function Fixture() {
  const [open, setOpen] = useState(false);
  const [nested, setNested] = useState(false);
  const [nativeNested, setNativeNested] = useState(false);
  const [nativeMounted, setNativeMounted] = useState(true);
  const [palette, setPalette] = useState(false);
  const [review, setReview] = useState("");
  const [notificationActions, setNotificationActions] = useState(0);
  const native = useRef<HTMLDialogElement>(null);
  return <>
    <AppToaster />
    <button onClick={() => notify.loading("浮层测试消息", {id: "overlay-task", actions: [
      {id: "stop", label: "通知操作", icon: <span>□</span>, onClick: () => setNotificationActions(count => count + 1)},
    ]})}>显示测试消息</button>
    <output aria-label="通知操作次数">{notificationActions}</output>
    <button onClick={() => setOpen(true)}>打开弹窗</button>
    <button onClick={() => native.current?.showModal()}>打开原生弹窗</button>
    <button onClick={() => setPalette(true)}>打开命令</button>
    <div className="fixture-clip"><Controls name="页面" /></div>
    <Dialog open={open} onClose={() => setOpen(false)} className="fixture-dialog" backdrop={<div className="fixture-backdrop" />}>
      <DialogHeading>父弹窗</DialogHeading>
      <button onClick={() => setOpen(false)}>关闭父弹窗</button>
      <div className="fixture-clip"><Controls name="父" onNested={() => setNested(true)} /></div>
      <StructuredReviewControls count={5} active={1} busy={false} metadataActive={false} onMove={() => {}}
        onResolve={(action, scope) => setReview(`${action}:${scope}`)} onKeepUnreviewed={() => setReview("unreviewed")} />
      <output aria-label="审阅结果">{review}</output>
      <Dialog open={nested} onClose={() => setNested(false)} className="fixture-dialog fixture-child" backdrop={<div className="fixture-backdrop" />}>
        <DialogHeading>子弹窗</DialogHeading><Controls name="子" />
        <button onClick={() => setNested(false)}>关闭子弹窗</button>
      </Dialog>
    </Dialog>
    {nativeMounted && <dialog ref={native} aria-label="原生弹窗"><div className="fixture-clip"><Controls name="原生" /></div>
      <Dialog open={nativeNested} onClose={() => setNativeNested(false)} className="fixture-dialog fixture-child" backdrop={<div className="fixture-backdrop" />}>
        <DialogHeading>原生内子弹窗</DialogHeading><Controls name="原生子" />
      </Dialog>
      <button onClick={() => setNativeNested(true)}>原生内打开子弹窗</button>
      <button onClick={() => native.current?.close()}>关闭原生弹窗</button>
      <button onClick={() => setNativeMounted(false)}>移除原生弹窗</button>
    </dialog>}
    {palette && <WritingCommandPalette onClose={() => setPalette(false)} commands={[{id: "run", label: "执行命令", description: "测试焦点与关闭", run: () => setReview("command")}]} />}
  </>;
}

createRoot(document.getElementById("root")!).render(<StrictMode><Fixture /></StrictMode>);
