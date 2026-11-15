import { Shimmer, SHIMMER_DEFAULTS, type ShimmerProps } from "./Shimmer";

import "./ShimmerText.css";

export const SHIMMER_TEXT_DEFAULTS = SHIMMER_DEFAULTS;

export interface ShimmerTextProps extends Omit<ShimmerProps, "children" | "as"> {
  readonly children: string;
}

/** Text convenience wrapper with wrapping rules; shares the generic UI shimmer. */
export function ShimmerText({ className, ...props }: ShimmerTextProps): React.JSX.Element {
  return <Shimmer {...props} className={`shimmer-text${className ? ` ${className}` : ""}`} />;
}
