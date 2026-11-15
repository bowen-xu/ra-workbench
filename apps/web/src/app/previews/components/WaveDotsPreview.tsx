import { useState } from "react";

import { WAVE_DOTS_DEFAULTS, WaveDots } from "../../../core/components/WaveDots";

interface Parameters {
  readonly durationSeconds: number;
  readonly waveDurationSeconds: number;
  readonly startDirection: "up" | "down";
  readonly amplitudePx: number;
  readonly dotSizePx: number;
  readonly gapPx: number;
}

const controls = [
  { key: "durationSeconds", label: "总时长（秒）", min: 0.1, max: 10, step: 0.05 },
  { key: "waveDurationSeconds", label: "波浪时间（秒）", min: 0, max: 10, step: 0.05 },
  { key: "amplitudePx", label: "起伏幅度（px）", min: 0, max: 20, step: 0.5 },
  { key: "dotSizePx", label: "圆点直径（px）", min: 1, max: 24, step: 1 },
  { key: "gapPx", label: "圆点间距（px）", min: 0, max: 24, step: 1 },
] as const;

export function WaveDotsPreview(): React.JSX.Element {
  const [parameters, setParameters] = useState<Parameters>(WAVE_DOTS_DEFAULTS);
  const [active, setActive] = useState(true);
  return <section className="motion-demo-effect" aria-label="波浪圆点">
    <h2>波浪圆点</h2>
    <div className="shimmer-text-demo-layout">
      <section className="shimmer-text-demo-controls" aria-label="圆点参数">
        <label>启动方向<select value={parameters.startDirection} onChange={event => {
          const direction = event.currentTarget.value;
          if (direction === "up" || direction === "down") setParameters(current => ({ ...current, startDirection: direction }));
        }}><option value="up">先向上</option><option value="down">先向下</option></select></label>
        {controls.map(control => <label key={control.key}>{control.label}<input type="number"
          min={control.min} max={control.key === "waveDurationSeconds" ? parameters.durationSeconds : control.max} step={control.step} value={parameters[control.key]}
          onChange={event => {
            const value = event.currentTarget.valueAsNumber;
            if (Number.isFinite(value)) setParameters(current => {
              const max = control.key === "waveDurationSeconds" ? current.durationSeconds : control.max;
              const next = Math.min(max, Math.max(control.min, value));
              return { ...current, [control.key]: next,
                waveDurationSeconds: control.key === "durationSeconds" ? Math.min(current.waveDurationSeconds, next)
                  : control.key === "waveDurationSeconds" ? next : current.waveDurationSeconds };
            });
          }} /></label>)}
        <p className="shimmer-text-demo-caption">波浪时间是三个点完成整段波浪的时间，不能超过总时长。剩余时间三个点一起静止；两者相等时连续播放。幅度表示从原位向上、向下的最大距离。</p>
        <div className="shimmer-text-demo-actions">
          <button type="button" onClick={() => setActive(value => !value)}>{active ? "停止动画" : "播放动画"}</button>
          <button type="button" onClick={() => { setParameters(WAVE_DOTS_DEFAULTS); setActive(true); }}>恢复默认</button>
        </div>
      </section>
      <section className="shimmer-text-demo-preview" aria-label="圆点效果预览">
        <h3>效果预览</h3>
        <div className="shimmer-text-demo-stage"><WaveDots {...parameters} active={active} /></div>
        <dl><div><dt>总时长</dt><dd>{parameters.durationSeconds} 秒</dd></div>
          <div><dt>波浪时间</dt><dd>{parameters.waveDurationSeconds} 秒</dd></div>
          <div><dt>静止时间</dt><dd>{Number((parameters.durationSeconds - parameters.waveDurationSeconds).toFixed(2))} 秒</dd></div>
          <div><dt>上下幅度</dt><dd>±{parameters.amplitudePx} px</dd></div></dl>
        <p className="shimmer-text-demo-caption">与聊天窗“回到底部”按钮共用组件。系统启用“减少动态效果”时圆点静止。</p>
      </section>
    </div>
  </section>;
}
