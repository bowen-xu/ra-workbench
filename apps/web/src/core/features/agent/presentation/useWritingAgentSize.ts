import { useEffect, useRef, useState, type PointerEvent } from "react";
import { getAppStorage } from "../../../public/storage";

const MIN_HEIGHT = 220;
const PANEL_SIZE_STORAGE_KEY = "research-assistant.writing-agent.panel-size";
const SIZE_PRESETS = [
  { id: "compact", ratio: 0.4, label: "窄" },
  { id: "standard", ratio: 0.5, label: "标准" },
  { id: "comfortable", ratio: 0.6, label: "舒适" },
  { id: "wide", ratio: 0.7, label: "宽" },
  { id: "maximum", ratio: 0.8, label: "最大" },
] as const;
const MIN_WIDTH = 320;
const MAX_VIEWPORT_RATIO = 0.8;
export type WritingAgentSizePreset = typeof SIZE_PRESETS[number]["id"];

interface WritingAgentPanelSizePreference {
  readonly widthPreset?: WritingAgentSizePreset;
  readonly heightPreset?: WritingAgentSizePreset;
}

function isSizePreset(value: unknown): value is WritingAgentSizePreset {
  return typeof value === "string" && SIZE_PRESETS.some((preset) => preset.id === value);
}

function readPanelSizePreference(): WritingAgentPanelSizePreference {
  if (typeof window === "undefined") return {};
  try {
    const raw = getAppStorage().getItem(PANEL_SIZE_STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return {};
    const record = parsed as Record<string, unknown>;
    return {
      ...(isSizePreset(record.widthPreset) ? { widthPreset: record.widthPreset } : {}),
      ...(isSizePreset(record.heightPreset) ? { heightPreset: record.heightPreset } : {}),
    };
  } catch {
    return {};
  }
}

function writePanelSizePreference(preference: WritingAgentPanelSizePreference): void {
  if (typeof window === "undefined") return;
  try {
    getAppStorage().setItem(PANEL_SIZE_STORAGE_KEY, JSON.stringify(preference));
  } catch {
    // Storage may be unavailable in a restricted desktop webview. The panel
    // remains usable for the current session in that case.
  }
}

function viewportSize(): { readonly width: number; readonly height: number } {
  if (typeof window === "undefined") return { width: 1280, height: 800 };
  return { width: window.innerWidth || 1280, height: window.innerHeight || 800 };
}

function valueForPreset(preset: WritingAgentSizePreset, axis: "width" | "height", viewport: { readonly width: number; readonly height: number }): number {
  const ratio = SIZE_PRESETS.find((candidate) => candidate.id === preset)?.ratio ?? 0.5;
  const value = Math.floor((axis === "width" ? viewport.width : viewport.height) * ratio);
  return axis === "width" ? Math.max(MIN_WIDTH, value) : Math.max(MIN_HEIGHT, value);
}

function nearestPreset(value: number, axis: "width" | "height", viewport: { readonly width: number; readonly height: number }): WritingAgentSizePreset {
  return SIZE_PRESETS.reduce<WritingAgentSizePreset>((best, preset) => {
    const bestDistance = Math.abs(valueForPreset(best, axis, viewport) - value);
    const distance = Math.abs(valueForPreset(preset.id, axis, viewport) - value);
    return distance < bestDistance ? preset.id : best;
  }, SIZE_PRESETS[1].id);
}

export function useWritingAgentSize(defaultSize?: WritingAgentSizePreset, rightEdgeEnabled = true) {
  const savedSize = readPanelSizePreference();
  const [viewport, setViewport] = useState(viewportSize);
  const [heightPreset, setHeightPreset] = useState<WritingAgentSizePreset>(defaultSize ?? savedSize.heightPreset ?? "standard");
  const [widthPreset, setWidthPreset] = useState<WritingAgentSizePreset>(defaultSize ?? savedSize.widthPreset ?? "standard");
  const resizeRef = useRef<{ readonly pointerId: number; readonly startHeight: number; readonly startY: number } | undefined>(undefined);
  const widthResizeRef = useRef<{ readonly pointerId: number; readonly startWidth: number; readonly startX: number; readonly side: "left" | "right" } | undefined>(undefined);
  const cornerResizeRef = useRef<{ readonly pointerId: number; readonly startWidth: number; readonly startHeight: number; readonly startX: number; readonly startY: number; readonly side: "left" | "right" } | undefined>(undefined);
  const [widthResizing, setWidthResizing] = useState(false);
  const currentWidth = {
    value: valueForPreset(widthPreset, "width", viewport),
    label: SIZE_PRESETS.find((preset) => preset.id === widthPreset)?.label ?? "标准",
  };
  const currentHeight = valueForPreset(heightPreset, "height", viewport);
  const maximumWidth = Math.max(MIN_WIDTH, Math.floor(viewport.width * MAX_VIEWPORT_RATIO));
  const maximumHeight = Math.max(MIN_HEIGHT, Math.floor(viewport.height * MAX_VIEWPORT_RATIO));

  useEffect(() => {
    const onViewportResize = () => setViewport(viewportSize());
    window.addEventListener("resize", onViewportResize);
    return () => window.removeEventListener("resize", onViewportResize);
  }, []);

  useEffect(() => {
    writePanelSizePreference({ widthPreset, heightPreset });
  }, [heightPreset, widthPreset]);



  function onResizePointerDown(event: PointerEvent<HTMLDivElement>): void {
    event.preventDefault();
    resizeRef.current = { pointerId: event.pointerId, startHeight: currentHeight, startY: event.clientY };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onResizePointerMove(event: PointerEvent<HTMLDivElement>): void {
    const resize = resizeRef.current;
    if (resize === undefined || resize.pointerId !== event.pointerId) return;
    setHeightPreset(nearestPreset(Math.min(maximumHeight, Math.max(MIN_HEIGHT, resize.startHeight + resize.startY - event.clientY)), "height", viewport));
  }

  function onResizePointerUp(event: PointerEvent<HTMLDivElement>): void {
    if (resizeRef.current?.pointerId !== event.pointerId) return;
    resizeRef.current = undefined;
    event.currentTarget.releasePointerCapture?.(event.pointerId);
  }

  function adjustWidth(direction: -1 | 1): void {
    const index = SIZE_PRESETS.findIndex((preset) => preset.id === widthPreset);
    const next = Math.min(SIZE_PRESETS.length - 1, Math.max(0, index + direction));
    setWidthPreset(SIZE_PRESETS[next]?.id ?? "standard");
  }

  function onWidthResizePointerDown(side: "left" | "right", event: PointerEvent<HTMLElement>): void {
    if (side === "right" && !rightEdgeEnabled) return;
    event.preventDefault();
    widthResizeRef.current = { pointerId: event.pointerId, startWidth: currentWidth.value, startX: event.clientX, side };
    setWidthResizing(true);
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onWidthResizePointerMove(event: PointerEvent<HTMLElement>): void {
    const resize = widthResizeRef.current;
    if (resize === undefined || resize.pointerId !== event.pointerId) return;
    if (resize.side === "right" && !rightEdgeEnabled) {onWidthResizePointerUp(event); return;}
    const delta = event.clientX - resize.startX;
    const desiredWidth = resize.startWidth + (resize.side === "right" ? delta : -delta);
    setWidthPreset(nearestPreset(Math.min(maximumWidth, Math.max(MIN_WIDTH, desiredWidth)), "width", viewport));
  }

  function onWidthResizePointerUp(event: PointerEvent<HTMLElement>): void {
    if (widthResizeRef.current?.pointerId !== event.pointerId) return;
    widthResizeRef.current = undefined;
    setWidthResizing(false);
    event.currentTarget.releasePointerCapture?.(event.pointerId);
  }

  function onCornerResizePointerDown(side: "left" | "right", event: PointerEvent<HTMLElement>): void {
    if (side === "right" && !rightEdgeEnabled) return;
    event.preventDefault();
    cornerResizeRef.current = {
      pointerId: event.pointerId,
      startWidth: currentWidth.value,
      startHeight: currentHeight,
      startX: event.clientX,
      startY: event.clientY,
      side,
    };
    setWidthResizing(true);
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onCornerResizePointerMove(event: PointerEvent<HTMLElement>): void {
    const resize = cornerResizeRef.current;
    if (resize === undefined || resize.pointerId !== event.pointerId) return;
    if (resize.side === "right" && !rightEdgeEnabled) {onCornerResizePointerUp(event); return;}
    const deltaX = event.clientX - resize.startX;
    const deltaY = event.clientY - resize.startY;
    const desiredWidth = resize.startWidth + (resize.side === "right" ? deltaX : -deltaX);
    const desiredHeight = resize.startHeight - deltaY;
    setWidthPreset(nearestPreset(Math.min(maximumWidth, Math.max(MIN_WIDTH, desiredWidth)), "width", viewport));
    setHeightPreset(nearestPreset(Math.min(maximumHeight, Math.max(MIN_HEIGHT, desiredHeight)), "height", viewport));
  }

  function onCornerResizePointerUp(event: PointerEvent<HTMLElement>): void {
    if (cornerResizeRef.current?.pointerId !== event.pointerId) return;
    cornerResizeRef.current = undefined;
    setWidthResizing(false);
    event.currentTarget.releasePointerCapture?.(event.pointerId);
  }

  function adjustHeight(direction: -1 | 1): void {
    const index = SIZE_PRESETS.findIndex((preset) => preset.id === heightPreset);
    const next = Math.min(SIZE_PRESETS.length - 1, Math.max(0, index + direction));
    setHeightPreset(SIZE_PRESETS[next]?.id ?? "standard");
  }
  function borderAt(event: PointerEvent<HTMLElement>): {side: "left" | "right"; corner: boolean} | undefined {
    // The panel's own border is the hit target. Descendants include the native
    // scroll viewport, so never resize from a bubbled content/scrollbar event.
    if (event.target !== event.currentTarget) return;
    const panel = event.currentTarget, rect = panel.getBoundingClientRect();
    if (!rect.width || !panel.offsetWidth) return;
    const scale = rect.width / panel.offsetWidth;
    const style = window.getComputedStyle(panel);
    const left = (parseFloat(style.borderLeftWidth) || 0) * scale;
    const right = (parseFloat(style.borderRightWidth) || 0) * scale;
    const side = event.clientX < rect.left + left ? "left" : event.clientX >= rect.right - right ? "right" : undefined;
    if (side === "right" && !rightEdgeEnabled) return;
    return side ? {side, corner: event.clientY < rect.top + 10} : undefined;
  }
  function finishEdgeResize(event: PointerEvent<HTMLElement>): void {
    onWidthResizePointerUp(event); onCornerResizePointerUp(event);
    delete event.currentTarget.dataset.resizeEdge;
  }
  const edgeResizeProps = {
    onPointerDown(event: PointerEvent<HTMLElement>) {
      if (event.button !== 0) return;
      const edge = borderAt(event);
      if (edge?.corner) onCornerResizePointerDown(edge.side, event);
      else if (edge) onWidthResizePointerDown(edge.side, event);
    },
    onPointerMove(event: PointerEvent<HTMLElement>) {
      if (widthResizeRef.current || cornerResizeRef.current) {
        onWidthResizePointerMove(event); onCornerResizePointerMove(event); return;
      }
      const edge = borderAt(event);
      if (edge) event.currentTarget.dataset.resizeEdge = edge.corner ? edge.side : "width";
      else delete event.currentTarget.dataset.resizeEdge;
    },
    onPointerLeave(event: PointerEvent<HTMLElement>) {
      if (!widthResizeRef.current && !cornerResizeRef.current) delete event.currentTarget.dataset.resizeEdge;
    },
    onPointerUp: finishEdgeResize,
    onPointerCancel: finishEdgeResize,
    onLostPointerCapture: finishEdgeResize,
  };
  return { heightPreset, widthPreset, widthResizing, currentWidth, currentHeight, maximumWidth, maximumHeight,
    minimumWidth: MIN_WIDTH, minimumHeight: MIN_HEIGHT, adjustWidth, adjustHeight,
    onResizePointerDown, onResizePointerMove, onResizePointerUp,
    edgeResizeProps };
}
