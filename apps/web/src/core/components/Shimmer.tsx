import type { ComponentPropsWithoutRef, CSSProperties } from "react";

import "./Shimmer.css";

export const SHIMMER_DEFAULTS = {
  maskWidthPercent: 50,
  centerOpacityPercent: 15,
  sweepDurationSeconds: 0.5,
  intervalSeconds: 3,
} as const;

export interface ShimmerProps extends ComponentPropsWithoutRef<"span"> {
  /** Use a div for block content; span is the default inline wrapper. */
  readonly as?: "span" | "div";
  readonly active?: boolean;
  /** Full gradient band width as a percentage of the wrapper width. Zero disables shimmer. */
  readonly maskWidthPercent?: number;
  /** Center opacity from 0 (invisible) to 100 (opaque). */
  readonly centerOpacityPercent?: number;
  /** Seconds for the entire band to enter at the left and fully exit at the right. Must be positive. */
  readonly sweepDurationSeconds?: number;
  /** Static pause between sweeps, in seconds. Zero gives continuous sweeps. */
  readonly intervalSeconds?: number;
}

type ShimmerStyle = CSSProperties & {
  readonly "--shimmer-mask-size": string;
  readonly "--shimmer-band-start": string;
  readonly "--shimmer-band-end": string;
  readonly "--shimmer-center-opacity": number;
  readonly "--shimmer-cycle-duration": string;
  readonly "--shimmer-timing": string;
};

function finiteOrDefault(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

/** Masks the whole UI subtree without changing its interaction or announcing loading state. */
export function Shimmer({ children, as: Tag = "span", active = true, className, style,
  maskWidthPercent = SHIMMER_DEFAULTS.maskWidthPercent,
  centerOpacityPercent = SHIMMER_DEFAULTS.centerOpacityPercent,
  sweepDurationSeconds = SHIMMER_DEFAULTS.sweepDurationSeconds,
  intervalSeconds = SHIMMER_DEFAULTS.intervalSeconds,
  ...props }: ShimmerProps): React.JSX.Element {
  const bandRatio = Math.max(0, finiteOrDefault(maskWidthPercent, SHIMMER_DEFAULTS.maskWidthPercent)) / 100;
  const opacity = Math.min(100, Math.max(0, finiteOrDefault(centerOpacityPercent, SHIMMER_DEFAULTS.centerOpacityPercent))) / 100;
  const requestedSweep = finiteOrDefault(sweepDurationSeconds, SHIMMER_DEFAULTS.sweepDurationSeconds);
  const sweep = requestedSweep > 0 ? requestedSweep : SHIMMER_DEFAULTS.sweepDurationSeconds;
  const interval = Math.max(0, finiteOrDefault(intervalSeconds, SHIMMER_DEFAULTS.intervalSeconds));
  // At mask-position 100% and 0%, a centered band just touches the wrapper's left
  // and right edges. Thus the whole visible sweep takes exactly `sweep` seconds.
  const canvasRatio = 2 + bandRatio;
  const bandHalfPercent = bandRatio / canvasRatio * 50;
  const cycle = sweep + interval;
  const animationStyle: ShimmerStyle = {
    ...style,
    "--shimmer-mask-size": `${canvasRatio * 100}%`,
    "--shimmer-band-start": `${50 - bandHalfPercent}%`,
    "--shimmer-band-end": `${50 + bandHalfPercent}%`,
    "--shimmer-center-opacity": opacity,
    "--shimmer-cycle-duration": `${cycle}s`,
    "--shimmer-timing": interval === 0 ? "linear" : `linear(0, 1 ${sweep / cycle * 100}%, 1)`,
  };
  return <Tag {...props} className={`shimmer${className ? ` ${className}` : ""}`}
    style={animationStyle} data-shimmer-active={active && bandRatio > 0 ? "true" : "false"}>{children}</Tag>;
}
