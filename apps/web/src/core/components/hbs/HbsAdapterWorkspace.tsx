import { useEffect, useRef, useSyncExternalStore, type ReactNode } from "react";
import type { HbsSession } from "@research-assistant/core/hbs";
import type { HbsDeliveryPort } from "./deliveryPort";
import { ActionMenu, ActionMenuItem } from "../ActionMenu";
import "./HbsAdapterWorkspace.css";
import { HbsOperationLogPanel } from "./HbsOperationLogPanel";
import { operationTarget, type HbsOperationLog } from "./operationLog";

/** Inspectable production composition; only lifecycle, fixture operations and
 * the port are supplied by a preview host. */
export function HbsAdapterWorkspace<D>({ session, delivery, authoritative, renderSnapshot, renderEditor, onExternal, externalActions, onRestart, onReset, controls, operations, resourceId }: {
  readonly operations?: HbsOperationLog; readonly resourceId?: string;
  readonly session: HbsSession<D>; readonly delivery: HbsDeliveryPort<D>; readonly authoritative: { readonly saved: D; readonly baseline: D; readonly version: number };
  readonly renderSnapshot: (value: D, label: string) => ReactNode; readonly renderEditor: () => ReactNode;
  readonly onExternal: (id: string) => void;
  readonly externalActions: readonly { readonly id: string; readonly label: string; readonly group?: string }[];
  readonly onRestart: () => void; readonly onReset: () => void; readonly controls?: ReactNode;
}): React.JSX.Element {
  const state = useSyncExternalStore(session.subscribe, session.getSnapshot), transport = useSyncExternalStore(delivery.subscribe, delivery.getSnapshot);
  const frame = useRef<HTMLElement>(null);
  useEffect(() => operations?.observe(session, resourceId ?? "unknown"), [operations, session, resourceId]);
  useEffect(() => operations?.observeDelivery(delivery, resourceId ?? "unknown"), [operations, delivery, resourceId]);
  useEffect(() => {
    const element = frame.current;
    if (!element || !operations) return;
    const beforeInput = (event: InputEvent) => operations.record("文档编辑触发", {kind: session.adapter.kind, resourceId,
      version: session.getSnapshot().observed.version, inputType: event.inputType, data: event.data, composing: event.isComposing, target: operationTarget(event.target)});
    element.addEventListener("beforeinput", beforeInput, true);
    return () => element.removeEventListener("beforeinput", beforeInput, true);
  }, [operations, session, resourceId]);
  const context = () => ({kind: session.adapter.kind, resourceId, version: session.getSnapshot().observed.version});
  return <main className="hbs-adapter-workspace" onClickCapture={event => {
    const target = event.target instanceof Element ? event.target.closest("button") : null;
    if (target && !target.closest(".hbs-operation-log")) operations?.record("点击操作", {...context(), action: target.getAttribute("aria-label") ?? target.textContent?.trim(), target: operationTarget(target)});
  }} onChangeCapture={event => {
    const target = event.target;
    if (target instanceof HTMLSelectElement && target.getAttribute("aria-label") === "文档类型") operations?.record("切换文档类型", {...context(), value: target.value});
    if (target instanceof HTMLInputElement && target.type === "checkbox" && target.closest(".hbs-workspace-actions")) operations?.record("切换自动投递", {...context(), automatic: target.checked});
  }} onKeyDownCapture={event => {
    if (!(event.target instanceof Element) || !event.target.closest(".hbs-editor-frame")) return;
    const key = event.key.toLowerCase();
    const history = (event.metaKey || event.ctrlKey) && (key === "z" || key === "y");
    if (history || ["Backspace", "Delete", "Enter"].includes(event.key)) operations?.record(history ? (key === "y" || event.shiftKey ? "触发重做" : "触发撤销") : `按键 ${event.key}`,
      {...context(), key: event.key, meta: event.metaKey, ctrl: event.ctrlKey, shift: event.shiftKey, alt: event.altKey, target: operationTarget(event.target)});
  }}>
    <header><div><h1>HBS 文档适配器</h1><p>真实编辑器 · IndexedDB 持久化 · 合成初始内容</p></div>{controls}</header>
    <div className="hbs-workspace-actions">
      {externalActions.filter(action=>!action.group).map(action=><button key={action.id} onClick={()=>onExternal(action.id)}>{action.label}</button>)}
      {[...new Set(externalActions.flatMap(action=>action.group?[action.group]:[]))].map(group=><ActionMenu key={group} label={`修改 S：${group}`} trigger={`修改 S：${group}`}
        className="hbs-external-menu" contentClassName="hbs-external-actions" portal>
        {externalActions.filter(action=>action.group===group).map(action=><ActionMenuItem key={action.id} onClick={()=>onExternal(action.id)}>{action.label}</ActionMenuItem>)}
      </ActionMenu>)}
      <button onClick={() => { void session.syncLatest(); }}>同步 S → H</button>
      <button onClick={() => { void delivery.deliver(); }}>发送 S ← H</button><button onClick={() => { void session.save(); }}>保存 H</button>
      <label><input type="checkbox" role="switch" checked={transport.automatic} onChange={event => delivery.setAutomatic(event.target.checked)} />自动投递到 S</label>
      <button onClick={onRestart}>模拟重启恢复 H</button><button onClick={onReset}>重置当前资源</button>
    </div>
    <p role="status">{state.message} · 待发送 {transport.waiting} · 排队 {state.queued} · 前端 v{state.observed.version} / S v{authoritative.version}</p>
    {state.preservationError && <p role="alert">{state.preservationError}</p>}
    <div className="hbs-observations">{([
      ["S", "权威保存内容", authoritative.saved], ["B", "后端已确认基线", authoritative.baseline],
      ["S′", "持久化编辑保全；保存不代表 Keep", state.preserved], ["B′", "审阅基线（推导，未落库）", state.baselineProjection],
    ] as const).map(([title, subtitle, value]) => <section className="hbs-observation" key={title} aria-label={title}>
      <header><b>{title}</b><span>{subtitle}</span></header><div className="hbs-snapshot-frame">{value===undefined ? <p>尚未成功保全</p> : renderSnapshot(value, title)}</div>
    </section>)}</div>
    <section ref={frame} className="hbs-editor-frame" aria-label="H" onInputCapture={event => {
      const input = event.nativeEvent as InputEvent;
      operations?.record("文档输入／删除事件", {...context(), inputType: input.inputType, data: input.data, composing: input.isComposing, target: operationTarget(event.target)});
    }} onCompositionStartCapture={event => operations?.record("开始组合输入", {...context(), target: operationTarget(event.target)})}
      onCompositionEndCapture={event => operations?.record("结束组合输入", {...context(), data: event.data, target: operationTarget(event.target)})}>
      <header><b>H</b><span>当前显示内容 · {state.changes.length} 处待审 · {state.conflicts.length} 处冲突{state.pending ? " · 等待保存回执" : ""}</span></header>{renderEditor()}</section>
    {operations && <HbsOperationLogPanel log={operations} />}
    <details><summary>消息通道与记录</summary><ol>{transport.events.map((event, index) => <li key={index}>{event}</li>)}</ol><ol>{state.events.map((event, index) => <li key={index}>{event}</li>)}</ol></details>
  </main>;
}
