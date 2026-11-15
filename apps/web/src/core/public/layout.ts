/** Shared workbench capability; implementation and lifecycle remain owned by Core. */
export { retainRowSelectionIds, updateRowSelection } from "../components/selection/rowSelection";
export type { RowSelectionInput, RowSelectionResult } from "../components/selection/rowSelection";
export { useVisibleLayoutMeasurement, visibleLayoutRect } from "../components/layout/useVisibleLayoutMeasurement";
export { startMarqueeSelection } from "../components/selection/marqueeSelection";
export type { MarqueePoint } from "../components/selection/marqueeSelection";
export { createMarqueeHitTest } from "../components/selection/marqueeHitTest";
export { listWindowBoundary, listWindowOffsets, listWindowRange } from "../components/layout/listWindowGeometry";
export { useNestedScrollRegion, useNestedWheelScroll } from "../components/scroll/useNestedWheelScroll";
export { useScrollEdges } from "../components/scroll/useScrollEdges";
