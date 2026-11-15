import type { ReactNode } from "react";
import { RotateCcw } from "lucide-react";
import { DeveloperModeToggle } from "../../../core/features/settings/DeveloperMode";
import { SOURCE_NAVIGATION_PREVIEW_URL } from "../../../core/runtime/sourceNavigationPreview";
import { AgentCanvasTextSettings, type CanvasTextSettings } from "./AgentCanvasTextSettings";
import { agentCanvasScenarios } from "../../../core/features/agent/agentCanvasScenario";
import type { AgentCanvasScenario } from "../../../core/features/agent/agentCanvasScenario";

export interface CanvasExtraScene { readonly value: string; readonly label: string; readonly unavailable?: string }
export function CanvasControlGroup({ title, description, children }: {
  readonly title: string; readonly description?: string; readonly children: ReactNode;
}): React.JSX.Element {
  return <section className="agent-canvas-control-section" aria-label={title}>
    <div className="agent-canvas-control-heading"><h2>{title}</h2>{description && <p>{description}</p>}</div>
    <div className="agent-canvas-control-group">{children}</div>
  </section>;
}
/** Both backends supply callbacks to these same controls and DOM. */
export function AgentCanvasScenarioControls({ scenario, duration, settings, onScenario, onDuration, onSettings, onReset,
  disabled = false, settingsDisabled = false, extraScenes = [], unavailable = {}, children }: {
  readonly scenario: string; readonly duration: number; readonly settings: CanvasTextSettings;
  readonly onScenario: (value: string) => void; readonly onDuration: (minutes: number) => void;
  readonly onSettings: (settings: CanvasTextSettings) => void; readonly onReset: () => void;
  readonly disabled?: boolean; readonly extraScenes?: readonly CanvasExtraScene[]; readonly children?: ReactNode;
  readonly settingsDisabled?: boolean;
  readonly unavailable?: Partial<Record<AgentCanvasScenario, string>>;
}): React.JSX.Element {
  return <>
    <CanvasControlGroup title="预览场景" description="选择模拟响应，使用实际聊天和运行流程。">
    <div className="agent-canvas-control-row">
    <a className="agent-canvas-demo-reset" href={SOURCE_NAVIGATION_PREVIEW_URL}>来源引用预览</a>
    <button className="agent-canvas-demo-reset" type="button" disabled={disabled} onClick={onReset}><RotateCcw size={15} />重置假数据</button>
    </div><div className="agent-canvas-control-row">
    <label className="agent-canvas-demo-duration">模拟运行时长<select aria-label="模拟运行时长" disabled={disabled || settingsDisabled} value={duration} onChange={event => onDuration(Number(event.target.value))}>
      {[[1 / 60, "1 秒"], [.05, "3 秒"], [.5, "30 秒"], [5, "5 分钟"], [60, "1 小时"]].map(([value, label]) => <option key={value} value={value}>{label}</option>)}
    </select></label>
    <label className="agent-canvas-demo-duration">消息场景<select aria-label="模拟消息场景" disabled={disabled} value={scenario} onChange={event => onScenario(event.target.value)}>
      {agentCanvasScenarios.map(([value, label]) => <option key={value} value={value} disabled={!!unavailable[value]}>
        {label}{unavailable[value] ? `（${unavailable[value]}）` : ""}
      </option>)}
      {extraScenes.length > 0 && <optgroup label="原生调度校验">{extraScenes.map(scene => <option key={scene.value} value={scene.value} disabled={!!scene.unavailable}>
        {scene.label}{scene.unavailable ? `（${scene.unavailable}）` : ""}
      </option>)}</optgroup>}
    </select></label>
    </div></CanvasControlGroup>
    <CanvasControlGroup title="输出节奏" description="调整合成输出、渐显与压缩耗时。">
      <div className="agent-canvas-control-row"><AgentCanvasTextSettings settings={settings} onChange={onSettings} disabled={disabled || settingsDisabled} /></div>
    </CanvasControlGroup>
    {children}
    <CanvasControlGroup title="开发者选项"><div className="agent-canvas-control-row"><DeveloperModeToggle /></div></CanvasControlGroup>
  </>;
}
