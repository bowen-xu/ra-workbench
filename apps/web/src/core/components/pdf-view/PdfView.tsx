import { useEffect, useRef, useSyncExternalStore, type ReactNode, type RefObject, type PointerEventHandler } from "react";
import { createPortal } from "react-dom";
import type { PdfPagePoint, PdfPagePointerEvent, PdfSelection } from "@research-assistant/core/pdf-view";
import type { PdfPageHost } from "./PdfViewAdapter";
import { usePdfCharacterSelection } from "./usePdfCharacterSelection";
import { usePdfZoomGestures } from "./usePdfZoomGestures";
import { normalizedPointRect } from "./pdfCoordinates";
import type { SourceRect } from "@research-assistant/core/literature";
import "./PdfView.css";

import type { PdfViewController } from "./PdfViewController";

interface Props {
  readonly controller: PdfViewController;
  readonly containerRef: RefObject<HTMLDivElement | null>;
  readonly viewerRef: RefObject<HTMLDivElement | null>;
  readonly ready: boolean;
  readonly active: boolean;
  readonly characterSelectionAvailable: boolean;
  readonly pageHosts: readonly PdfPageHost[];
  readonly documentVersion: string;
  readonly selection?: PdfSelection | null;
  readonly onSelectionStart?: () => void;
  readonly onSelectionComplete?: (selection: PdfSelection) => void;
  readonly renderSelectionActions?: (selection: PdfSelection) => ReactNode;
  readonly onPagePointer?: (event: PdfPagePointerEvent) => void;
  readonly onPageDoubleClick?: (point: PdfPagePoint) => void;
  readonly onPageContextMenu?: (pageIndex: number, rect: SourceRect) => void;
  readonly onPointerMove?: PointerEventHandler<HTMLDivElement>;
  readonly onPointerLeave?: PointerEventHandler<HTMLDivElement>;
  readonly onScroll?: () => void;
  readonly renderPageOverlay?: (pageIndex: number) => ReactNode;
}

/** A composable document surface. Business overlays and selection actions stay in hosts. */
export function PdfView(props: Props): React.JSX.Element {
  const state = useSyncExternalStore(props.controller.subscribe, props.controller.getSnapshot);
  const setLocalSelection = (selection: PdfSelection | undefined): void => props.controller.setSelection(selection);
  const selection = props.selection === undefined ? state.selection : props.selection ?? undefined;
  const selectionRef = useRef(selection);
  selectionRef.current = selection;
  usePdfCharacterSelection({
    version: props.documentVersion,
    enabled: props.ready && props.active && props.characterSelectionAvailable,
    viewerRef: props.viewerRef, pageStore: props.controller.adapter.characterStore, pageHosts: props.pageHosts,
    doubleClickAction: props.onPageDoubleClick !== undefined,
    onSelectionStart: () => { setLocalSelection(undefined); props.onSelectionStart?.(); },
    onSelectionComplete: (value) => { setLocalSelection(value); props.onSelectionComplete?.(value); },
  });
  usePdfZoomGestures(props.ready && props.active, props.containerRef, props.controller.adapter);
  useEffect(() => {
    const container = props.containerRef.current;
    if (!container || !props.active || !props.ready || !props.onPageContextMenu) return;
    // Native bubbling also covers business overlays mounted through portals.
    const contextMenu = (event: MouseEvent): void => {
      if (isPdfEditable(event.target) || (event.target instanceof Element && event.target.closest(".saved-position-note"))) return;
      const point = props.controller.adapter.pagePoint(event.clientX, event.clientY);
      if (!point) return;
      const bounds = props.controller.adapter.clientRectForPageRect(point.pageIndex, { x: 0, y: 0, width: 1, height: 1 });
      const rect = bounds && normalizedPointRect(bounds, event.clientX, event.clientY);
      if (!rect) return;
      event.preventDefault(); event.stopPropagation(); setLocalSelection(undefined);
      props.onPageContextMenu?.(point.pageIndex, rect);
    };
    container.addEventListener("contextmenu", contextMenu);
    return () => container.removeEventListener("contextmenu", contextMenu);
  }, [props.active, props.ready, props.containerRef, props.controller, props.onPageContextMenu]);
  useEffect(() => { setLocalSelection(undefined); }, [props.documentVersion]);
  useEffect(() => {
    const container = props.containerRef.current;
    if (!container || !props.active) return;
    const copy = (event: ClipboardEvent): void => {
      if (!selectionRef.current?.text || isPdfEditable(event.target)) return;
      event.clipboardData?.setData("text/plain", selectionRef.current.text);
      event.preventDefault();
    };
    container.addEventListener("copy", copy);
    return () => container.removeEventListener("copy", copy);
  }, [props.active, props.containerRef]);
  const segments = selection?.segments ?? [];
  return <>
    <div className="pdf-view pdf-viewer-container" ref={props.containerRef} tabIndex={0} data-reader-focus-group="view"
      onPointerDownCapture={event => {if (props.active && event.button === 0 && !isPdfEditable(event.target)) props.controller.clearSelection();}}
      onPointerDown={(event) => {
        const point = props.controller.adapter.pagePoint(event.clientX, event.clientY);
        if (props.active && state.source && point) props.onPagePointer?.({ document: state.source, phase: "down", point });
      }}
      onPointerUp={(event) => {
        const point = props.controller.adapter.pagePoint(event.clientX, event.clientY);
        if (props.active && state.source && point) props.onPagePointer?.({ document: state.source, phase: "up", point });
      }}
      onPointerMove={props.onPointerMove} onPointerLeave={props.onPointerLeave} onScroll={props.onScroll}
      onDoubleClick={(event) => {
        if (!props.active || !props.onPageDoubleClick || (event.target as Element).closest("a,button,input")) return;
        const point = props.controller.adapter.pagePoint(event.clientX, event.clientY);
        if (point) { event.preventDefault(); props.onPageDoubleClick(point); }
      }}>
      <div className="pdfViewer" ref={props.viewerRef} />
    </div>
    {selection && props.renderSelectionActions?.(selection)}
    {props.pageHosts.map((host) => createPortal(<>
      {segments.filter((segment) => segment.pageIndex === host.pageIndex).flatMap((segment) => segment.rects).map((rect, index) =>
        <span key={index} aria-hidden="true" className="pending-selection-highlight pdf-committed-selection-highlight" style={{ left: `${rect.x * 100}%`, top: `${rect.y * 100}%`, width: `${rect.width * 100}%`, height: `${rect.height * 100}%` }} />)}
      {props.renderPageOverlay?.(host.pageIndex)}
    </>, host.element))}
  </>;
}

export function isPdfEditable(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("input,textarea,select,[contenteditable=true],[role=textbox]") !== null;
}
