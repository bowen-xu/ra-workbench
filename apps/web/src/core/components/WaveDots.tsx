import type { ComponentPropsWithoutRef, CSSProperties } from "react";

import "./WaveDots.css";

export const WAVE_DOTS_DEFAULTS = {
  durationSeconds: 1,
  waveDurationSeconds: 0.7,
  startDirection: "up",
  amplitudePx: 2,
  dotSizePx: 4,
  gapPx: 3,
} as const;

export interface WaveDotsProps extends Omit<ComponentPropsWithoutRef<"span">, "children"> {
  readonly active?: boolean;
  /** Total cycle duration, including the wave and the stationary interval. Must be positive. */
  readonly durationSeconds?: number;
  /** Time for all three dots to complete the wave, clamped to the total cycle duration. */
  readonly waveDurationSeconds?: number;
  readonly startDirection?: "up" | "down";
  /** Maximum displacement above and below the resting position, in pixels. */
  readonly amplitudePx?: number;
  readonly dotSizePx?: number;
  readonly gapPx?: number;
}

type WaveDotsStyle = CSSProperties & {
  readonly "--wave-dots-duration": string;
  readonly "--wave-dots-wave-duration": string;
  readonly "--wave-dots-timing": string;
  readonly "--wave-dots-delay": string;
  readonly "--wave-dots-amplitude": string;
  readonly "--wave-dots-size": string;
  readonly "--wave-dots-gap": string;
};

function finiteOrDefault(value: number, fallback: number): number {
  return Number.isFinite(value) ? value : fallback;
}

function waveTiming(duration: number, cycle: number, startDirection: "up" | "down"): string {
  // Map a sampled sine wave onto the moving part, then hold its resting value.
  // The transform has just two keyframes so this easing applies to the whole cycle.
  const points = Array.from({ length: 17 }, (_, index) => {
    const phase = index / 16;
    const sign = startDirection === "down" ? -1 : 1;
    const position = Number(((1 - sign * Math.sin(phase * 2 * Math.PI)) / 2).toFixed(6));
    const timePercent = Number((phase * duration / cycle * 100).toFixed(6));
    return `${position} ${timePercent}%`;
  });
  return `linear(${points.join(", ")}, 0.5)`;
}

/** Decorative activity dots; the containing control owns its accessible name. */
export function WaveDots({ active = true, className, style,
  durationSeconds = WAVE_DOTS_DEFAULTS.durationSeconds,
  waveDurationSeconds = WAVE_DOTS_DEFAULTS.waveDurationSeconds,
  startDirection = WAVE_DOTS_DEFAULTS.startDirection,
  amplitudePx = WAVE_DOTS_DEFAULTS.amplitudePx,
  dotSizePx = WAVE_DOTS_DEFAULTS.dotSizePx,
  gapPx = WAVE_DOTS_DEFAULTS.gapPx,
  ...props }: WaveDotsProps): React.JSX.Element {
  const requestedDuration = finiteOrDefault(durationSeconds, WAVE_DOTS_DEFAULTS.durationSeconds);
  const duration = requestedDuration > 0 ? requestedDuration : WAVE_DOTS_DEFAULTS.durationSeconds;
  const wave = Math.min(duration, Math.max(0, finiteOrDefault(waveDurationSeconds, WAVE_DOTS_DEFAULTS.waveDurationSeconds)));
  // Stagger is an internal proportion of the complete three-dot wave, so the
  // last dot always returns to rest exactly at `waveDurationSeconds`.
  const delay = wave * 0.15;
  const dotDuration = wave - 2 * delay;
  const size = finiteOrDefault(dotSizePx, WAVE_DOTS_DEFAULTS.dotSizePx);
  const animationStyle: WaveDotsStyle = {
    ...style,
    "--wave-dots-duration": `${duration}s`,
    "--wave-dots-wave-duration": `${wave}s`,
    "--wave-dots-timing": waveTiming(dotDuration, duration, startDirection),
    "--wave-dots-delay": `${delay}s`,
    "--wave-dots-amplitude": `${Math.max(0, finiteOrDefault(amplitudePx, WAVE_DOTS_DEFAULTS.amplitudePx))}px`,
    "--wave-dots-size": `${size > 0 ? size : WAVE_DOTS_DEFAULTS.dotSizePx}px`,
    "--wave-dots-gap": `${Math.max(0, finiteOrDefault(gapPx, WAVE_DOTS_DEFAULTS.gapPx))}px`,
  };
  return <span {...props} aria-hidden="true" className={`wave-dots${className ? ` ${className}` : ""}`}
    style={animationStyle} data-wave-dots-active={active && wave > 0}>
    <span /><span /><span />
  </span>;
}
