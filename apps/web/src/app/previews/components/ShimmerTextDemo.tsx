import { useState } from "react";
import { Sparkles } from "lucide-react";

import { Shimmer } from "../../../core/components/Shimmer";
import { SHIMMER_TEXT_DEFAULTS, ShimmerText } from "../../../core/components/ShimmerText";
import { WaveDotsPreview } from "./WaveDotsPreview";
import "./ShimmerTextDemo.css";

interface Parameters {
  readonly maskWidthPercent: number;
  readonly centerOpacityPercent: number;
  readonly sweepDurationSeconds: number;
  readonly intervalSeconds: number;
}
const controls = [
  { key: "maskWidthPercent", label: "遮罩宽度（%）", min: 0, max: 200, step: 5 },
  { key: "centerOpacityPercent", label: "中心不透明度（%）", min: 0, max: 100, step: 1 },
  { key: "sweepDurationSeconds", label: "扫过时间（秒）", min: 0.05, max: 10, step: 0.05 },
  { key: "intervalSeconds", label: "间隔时间（秒）", min: 0, max: 30, step: 0.1 },
] as const;

export function ShimmerTextDemo(): React.JSX.Element {
  const [text, setText] = useState("准备中……");
  const [parameters, setParameters] = useState<Parameters>(SHIMMER_TEXT_DEFAULTS);
  const [active, setActive] = useState(true);
  return <main className="shimmer-text-demo">
    <header><p className="shimmer-text-demo-eyebrow">UI MOTION</p><h1>动效预览</h1>
      <p>实时调整波浪圆点与扫光动效，比较不同参数的效果。</p></header>
    <WaveDotsPreview />
    <section className="motion-demo-effect" aria-label="扫光动效">
    <h2>扫光</h2>
    <div className="shimmer-text-demo-layout">
      <section className="shimmer-text-demo-controls" aria-label="扫光参数">
        <label>文字内容<textarea rows={3} value={text} onChange={event => setText(event.currentTarget.value)} /></label>
        {controls.map(control => <label key={control.key}>{control.label}<input type="number"
          min={control.min} max={control.max} step={control.step} value={parameters[control.key]}
          onChange={event => {
            const value = event.currentTarget.valueAsNumber;
            if (Number.isFinite(value)) setParameters(current => ({ ...current, [control.key]: Math.min(control.max, Math.max(control.min, value)) }));
          }} /></label>)}
        <p className="shimmer-text-demo-caption">宽度相对于各示例的整个容器。15% 表示中心保留 15% 的不透明度；间隔时间不包含扫过时间。</p>
        <div className="shimmer-text-demo-actions">
          <button type="button" onClick={() => setActive(value => !value)}>{active ? "停止动画" : "播放动画"}</button>
          <button type="button" onClick={() => { setText("准备中……"); setParameters(SHIMMER_TEXT_DEFAULTS); setActive(true); }}>恢复默认</button>
        </div>
      </section>
      <section className="shimmer-text-demo-preview" aria-label="效果预览">
        <h3>效果预览</h3>
        <div className="shimmer-text-demo-stage"><ShimmerText {...parameters} active={active}>{text}</ShimmerText></div>
        <h3>整个标签（图标、文字、背景与边框）</h3>
        <div className="shimmer-text-demo-stage"><Shimmer {...parameters} active={active}>
          <span className="shimmer-text-demo-label"><Sparkles size={18} aria-hidden="true" /><span>{text}</span></span>
        </Shimmer></div>
        <dl><div><dt>遮罩画布宽度</dt><dd>{200 + parameters.maskWidthPercent}%</dd></div>
          <div><dt>每轮总时长</dt><dd>{Number((parameters.sweepDurationSeconds + parameters.intervalSeconds).toFixed(2))} 秒</dd></div></dl>
        <p className="shimmer-text-demo-caption">这是等待动效，不表示具体进度。系统启用“减少动态效果”时显示静态文字。</p>
      </section>
    </div>
    </section>
  </main>;
}
