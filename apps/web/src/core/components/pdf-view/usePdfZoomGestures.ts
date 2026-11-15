import { useEffect, type RefObject } from "react";

import { pdfWheelZoomFactorForInput } from "../pdfReaderModel";


interface WebKitGestureEvent extends Event {
  readonly clientX: number;
  readonly clientY: number;
  readonly scale: number;
}

export function usePdfZoomGestures(
  viewerReady: boolean,
  containerRef: RefObject<HTMLDivElement | null>,
  controller: { updateScaleAt(factor: number, origin: readonly [number, number]): void },
): void {
  useEffect(() => {
    const container = containerRef.current;
    if (!viewerReady || container === null) return;
    let gestureActive = false;
    let previousGestureScale = 1;
    let zoomFrame: number | undefined;
    let pendingScaleFactor = 1;
    let pendingOrigin: readonly [number, number] = [0, 0];

    const queueScaleAt = (scaleFactor: number, clientX: number, clientY: number): void => {
      if (!Number.isFinite(scaleFactor) || scaleFactor <= 0 || scaleFactor === 1) return;
      pendingScaleFactor *= scaleFactor;
      pendingOrigin = [clientX, clientY];
      if (zoomFrame !== undefined) return;
      zoomFrame = window.requestAnimationFrame(() => {
        zoomFrame = undefined;
        const frameScaleFactor = pendingScaleFactor;
        const frameOrigin = pendingOrigin;
        pendingScaleFactor = 1;
        controller.updateScaleAt(frameScaleFactor, frameOrigin);
      });
    };
    const handleWheel = (event: WheelEvent): void => {
      const scaleFactor = pdfWheelZoomFactorForInput({
        deltaY: event.deltaY,
        deltaMode: event.deltaMode,
        ctrlKey: event.ctrlKey,
        metaKey: event.metaKey,
        nativeGestureActive: gestureActive,
      });
      if (scaleFactor === undefined) return;
      event.preventDefault();
      queueScaleAt(scaleFactor, event.clientX, event.clientY);
    };
    const handleGestureStart = (event: Event): void => {
      const gesture = event as WebKitGestureEvent;
      gestureActive = true;
      previousGestureScale = gesture.scale || 1;
      event.preventDefault();
    };
    const handleGestureChange = (event: Event): void => {
      const gesture = event as WebKitGestureEvent;
      event.preventDefault();
      const nextScale = gesture.scale || previousGestureScale;
      queueScaleAt(nextScale / previousGestureScale, gesture.clientX, gesture.clientY);
      previousGestureScale = nextScale;
    };
    const handleGestureEnd = (event: Event): void => {
      gestureActive = false;
      previousGestureScale = 1;
      event.preventDefault();
    };

    container.addEventListener("wheel", handleWheel, { passive: false });
    container.addEventListener("gesturestart", handleGestureStart, { passive: false });
    container.addEventListener("gesturechange", handleGestureChange, { passive: false });
    container.addEventListener("gestureend", handleGestureEnd, { passive: false });
    return () => {
      if (zoomFrame !== undefined) window.cancelAnimationFrame(zoomFrame);
      container.removeEventListener("wheel", handleWheel);
      container.removeEventListener("gesturestart", handleGestureStart);
      container.removeEventListener("gesturechange", handleGestureChange);
      container.removeEventListener("gestureend", handleGestureEnd);
    };
  }, [containerRef, controller, viewerReady]);
}
