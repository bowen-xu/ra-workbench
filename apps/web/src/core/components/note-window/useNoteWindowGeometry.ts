import { useEffect, useRef, useState, type PointerEvent } from "react";
interface Geometry { x: number; y: number; width: number; height: number; }
const TOP = 35, GAP = 8;
export function clampNoteWindowGeometry(value: Geometry, width: number, height: number): Geometry {
  const w = Math.min(Math.max(320, value.width), Math.max(0, width - GAP * 2));
  const h = Math.min(Math.max(240, value.height), Math.max(0, height - TOP - GAP));
  return { width: w, height: h, x: Math.min(Math.max(GAP, value.x), Math.max(GAP, width - GAP - w)), y: Math.min(Math.max(TOP, value.y), Math.max(TOP, height - GAP - h)) };
}
export function resizeNoteWindowGeometry(base: Geometry, corner: "nw" | "ne" | "sw" | "se", dx: number, dy: number, width: number, height: number): Geometry {
  const west = corner.includes("w"), north = corner.includes("n");
  const minWidth = Math.min(320, Math.max(0, width - GAP * 2));
  const minHeight = Math.min(240, Math.max(0, height - TOP - GAP));
  const right = base.x + base.width, bottom = base.y + base.height;
  const left = west ? Math.min(Math.max(GAP, base.x + dx), right - minWidth) : base.x;
  const top = north ? Math.min(Math.max(TOP, base.y + dy), bottom - minHeight) : base.y;
  return clampNoteWindowGeometry({ x: left, y: top,
    width: west ? right - left : Math.min(Math.max(minWidth, base.width + dx), width - GAP - base.x),
    height: north ? bottom - top : Math.min(Math.max(minHeight, base.height + dy), height - GAP - base.y),
  }, width, height);
}

export function useNoteWindowGeometry() {
  const [geometry, setGeometry] = useState(() => clampNoteWindowGeometry({ x: (window.innerWidth - 520) / 2, y: (window.innerHeight - 400) / 2, width: 520, height: 400 }, window.innerWidth, window.innerHeight));
  const cleanup = useRef<(() => void) | undefined>(undefined);
  useEffect(() => {
    const resize = () => setGeometry(value => clampNoteWindowGeometry(value, window.innerWidth, window.innerHeight));
    window.addEventListener("resize", resize);
    return () => { window.removeEventListener("resize", resize); cleanup.current?.(); };
  }, []);
  function start(corner: "nw" | "ne" | "sw" | "se" | undefined, event: PointerEvent<HTMLElement>) {
    if (event.button !== 0 || (!corner && (event.target as HTMLElement).closest("button,input"))) return;
    event.preventDefault(); cleanup.current?.();
    const base = geometry, x = event.clientX, y = event.clientY, pointerId = event.pointerId;
    const move = (next: globalThis.PointerEvent) => {
      if (next.pointerId !== pointerId) return;
      const dx = next.clientX - x, dy = next.clientY - y;
      const candidate = corner ? resizeNoteWindowGeometry(base, corner, dx, dy, window.innerWidth, window.innerHeight)
        : { ...base, x: base.x + dx, y: base.y + dy };
      setGeometry(clampNoteWindowGeometry(candidate, window.innerWidth, window.innerHeight));
    };
    const end = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); window.removeEventListener("pointercancel", end); cleanup.current = undefined; };
    cleanup.current = end;
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", end); window.addEventListener("pointercancel", end);
  }
  return { style: { left: geometry.x, top: geometry.y, width: geometry.width, height: geometry.height }, startDrag: (event: PointerEvent<HTMLElement>) => start(undefined, event), startResize: start };
}
