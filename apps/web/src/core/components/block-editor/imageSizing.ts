import {useLayoutEffect, useState, type RefObject} from "react";

export const IMAGE_SCALE_STEP = 5;
export function isImageScale(value: unknown): value is number {
  return typeof value === "number" && value >= 5 && value <= 100 && value % IMAGE_SCALE_STEP === 0;
}
export function imageScaleForWidth(width: number, available: number): number {
  return Math.max(5, Math.min(100, Math.round(width / available * 100 / IMAGE_SCALE_STEP) * IMAGE_SCALE_STEP));
}

/** Measure the real block's available width, including nested and read-only hosts. */
export function useImageAvailableWidth(ref: RefObject<HTMLElement | null>, editor?: {readonly domElement: HTMLElement | undefined}): number {
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const parent = ref.current?.parentElement;
    const fallback = editor?.domElement?.firstElementChild;
    const measure = () => {
      const value = parent?.clientWidth || fallback?.clientWidth || 0;
      if (value > 0) setWidth(value);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    if (parent) observer.observe(parent);
    if (fallback && fallback !== parent) observer.observe(fallback);
    return () => observer.disconnect();
  }, [ref, editor]);
  return width;
}
