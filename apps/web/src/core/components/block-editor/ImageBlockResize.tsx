import {useLayoutEffect, useRef, useState, type PointerEvent, type ReactNode} from "react";
import {useEditorReadOnly} from "./EditorReadOnly";
import type {ImageAlignment} from "./ImageBlockIcons";
import {imageScaleForWidth} from "./imageSizing";

export interface ImageResizeSource {
  readonly blockId: string;
  readonly editor: {readonly domElement: HTMLElement | undefined; readonly isEditable: boolean};
  readonly onScaleChange: (scale: number) => void;
}
interface Gesture {
  readonly id: number;
  readonly target: HTMLElement;
  readonly startX: number;
  readonly startWidth: number;
  readonly factor: number;
  scale: number;
  changed: boolean;
}

/** One quantized gesture for every note host. Native pointer capture owns mouse,
 * touch and cancellation; only release/keyboard commits to the owner's history. */
export function ImageBlockResize({source, width, availableWidth, scale, alignment, onPreview, children}: {
  readonly source: ImageResizeSource;
  readonly width?: number | undefined;
  readonly availableWidth: number;
  readonly scale?: number | undefined;
  readonly alignment: ImageAlignment;
  readonly onPreview: (scale: number | undefined) => void;
  readonly children: ReactNode;
}): React.JSX.Element {
  const readOnly = useEditorReadOnly();
  const frame = useRef<HTMLDivElement>(null), gesture = useRef<Gesture | undefined>(undefined);
  const [dragging, setDragging] = useState(false);
  const [naturalStartWidth, setNaturalStartWidth] = useState<number>();
  const current = useRef({source, readOnly, onPreview}); current.current = {source, readOnly, onPreview};
  const cancel = () => {
    const active = gesture.current; gesture.current = undefined;
    if (active?.target.hasPointerCapture?.(active.id)) active.target.releasePointerCapture(active.id);
    setDragging(false); setNaturalStartWidth(undefined); current.current.onPreview(undefined);
  };
  useLayoutEffect(() => cancel, [source.blockId, availableWidth, alignment, scale]);
  const permitted = () => !current.current.readOnly && current.current.source.editor.isEditable;
  const update = (event: PointerEvent<HTMLElement>) => {
    const active = gesture.current;
    if (!active || active.id !== event.pointerId) return;
    if (!permitted()) {cancel(); return;}
    active.scale = imageScaleForWidth(active.startWidth + (event.clientX - active.startX) * active.factor, availableWidth);
    active.changed ||= event.clientX !== active.startX;
    current.current.onPreview(active.scale);
  };
  const value = gesture.current?.scale ?? scale ?? (availableWidth ? imageScaleForWidth(width ?? frame.current?.clientWidth ?? availableWidth, availableWidth) : 100);
  return <div ref={frame} className="note-image-resizable" data-resizing={dragging} style={{width: width ?? naturalStartWidth ?? "fit-content"}}>
    {children}
    {(["left", "right"] as const).map(side => <div key={side} className="note-image-resize-handle" data-side={side}
      role="slider" tabIndex={0} aria-label={`调整图片宽度（${side === "left" ? "左侧" : "右侧"}）`}
      aria-valuemin={5} aria-valuemax={100} aria-valuenow={value} aria-valuetext={`${value}%`} aria-orientation="horizontal"
      onPointerDown={event => {
        if (!permitted() || availableWidth <= 0 || event.button !== 0 || !event.isPrimary) return;
        event.preventDefault(); event.stopPropagation();
        event.currentTarget.focus({preventScroll: true});
        // Give an intrinsic legacy image a numeric visual starting point so
        // its first stepped width can transition too, without saving anything.
        const startWidth = width ?? (frame.current?.clientWidth || availableWidth);
        if (width === undefined) setNaturalStartWidth(startWidth);
        gesture.current = {id: event.pointerId, target: event.currentTarget, startX: event.clientX,
          // A new gesture starts at the logical size even if its preceding
          // visual transition is still between two discrete levels.
          startWidth,
          factor: (side === "left" ? -1 : 1) * (alignment === "center" ? 2 : 1), scale: value, changed: false};
        event.currentTarget.setPointerCapture(event.pointerId); setDragging(true);
      }}
      onPointerMove={update}
      onPointerUp={event => {
        if (gesture.current?.id !== event.pointerId) return;
        update(event);
        const active = gesture.current;
        if (active?.changed && permitted()) current.current.source.onScaleChange(active.scale);
        cancel();
      }}
      onPointerCancel={cancel} onLostPointerCapture={cancel}
      onKeyDownCapture={event => {
        if (event.key === "Escape" && gesture.current) {event.preventDefault(); event.stopPropagation(); cancel(); return;}
        if (!permitted() || availableWidth <= 0) return;
        const next = {ArrowLeft: value - 5, ArrowDown: value - 5, ArrowRight: value + 5, ArrowUp: value + 5, Home: 5, End: 100}[event.key];
        if (next === undefined) return;
        event.preventDefault(); event.stopPropagation();
        const level = Math.min(100, Math.max(5, next));
        if (level !== value) current.current.source.onScaleChange(level);
      }} />)}
  </div>;
}
