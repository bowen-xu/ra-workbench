import { useEffect, useState } from "react";
import { STREAM_REVEAL_DURATION_MS } from "../../../core/components/StreamReveal/StreamRevealTimeline";
import { DEFAULT_DEMO_TEXT_PACING, type WritingDemoTextPacing } from "./WritingDemoTextStream";

export interface CanvasTextSettings {
  readonly pacing: WritingDemoTextPacing;
  readonly revealDurationMs: number;
  readonly compactionDurationMs: number;
}
export const DEFAULT_CANVAS_TEXT_SETTINGS: CanvasTextSettings = {
  pacing: DEFAULT_DEMO_TEXT_PACING, revealDurationMs: STREAM_REVEAL_DURATION_MS,
  compactionDurationMs: 1000,
};

function NumberSetting({ label, value, min, step, unit, onChange, disabled }: {
  readonly label: string; readonly value: number; readonly min: number;
  readonly step: number; readonly unit: string; readonly onChange: (value: number) => void;
  readonly disabled: boolean;
}): React.JSX.Element {
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);
  return <label className="agent-canvas-demo-duration">{label}<input type="number" aria-label={label} disabled={disabled}
    value={draft} min={min} step={step} onBlur={() => setDraft(String(value))}
    onChange={event => {
      const text = event.currentTarget.value, next = Number(text);
      setDraft(text);
      if (text.trim() && Number.isFinite(next) && next >= min && (step !== 1 || Number.isInteger(next))) onChange(next);
    }} />{unit}</label>;
}

/** Controls mock emission, compaction latency and the scoped presentation clock. */
export function AgentCanvasTextSettings({ settings, onChange, disabled = false }: {
  readonly settings: CanvasTextSettings;
  readonly onChange: (settings: CanvasTextSettings) => void;
  readonly disabled?: boolean;
}): React.JSX.Element {
  return <>
    <NumberSetting label="每次输出字数" unit="字" min={1} step={1} disabled={disabled} value={settings.pacing.charactersPerChunk}
      onChange={charactersPerChunk => onChange({ ...settings, pacing: { ...settings.pacing, charactersPerChunk } })} />
    <NumberSetting label="输出间隔" unit="秒" min={.01} step={.01} disabled={disabled} value={settings.pacing.intervalMs / 1000}
      onChange={seconds => onChange({ ...settings, pacing: { ...settings.pacing, intervalMs: seconds * 1000 } })} />
    <NumberSetting label="渐显时长" unit="秒" min={0} step={.01} disabled={disabled} value={settings.revealDurationMs / 1000}
      onChange={seconds => onChange({ ...settings, revealDurationMs: seconds * 1000 })} />
    <NumberSetting label="模拟压缩时长" unit="秒" min={0} step={.1} disabled={disabled} value={settings.compactionDurationMs / 1000}
      onChange={seconds => onChange({ ...settings, compactionDurationMs: seconds * 1000 })} />
  </>;
}
