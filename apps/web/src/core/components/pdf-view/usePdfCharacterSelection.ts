import { useEffect, useLayoutEffect, useMemo, useRef, type RefObject } from "react";

import {
  PdfCharacterSelectionController,
} from "./pdfCharacterSelectionController";
import type { PdfCharacterSelection, PdfPageCharacterStore } from "./pdfPageCharacterModel";
import { PdfLiveSelectionPainter } from "./PdfLiveSelectionPainter";

interface UsePdfCharacterSelectionInput {
  readonly enabled: boolean;
  readonly version?: string;
  readonly doubleClickAction?: boolean;
  readonly viewerRef: RefObject<HTMLDivElement | null>;
  readonly pageStore: PdfPageCharacterStore;
  readonly pageHosts: readonly { readonly pageIndex: number; readonly element: HTMLElement }[];
  readonly onSelectionStart: () => void;
  readonly onSelectionComplete: (selection: PdfCharacterSelection) => void;
}

export function usePdfCharacterSelection({
  enabled,
  version,
  doubleClickAction = false,
  viewerRef,
  pageStore,
  pageHosts,
  onSelectionStart,
  onSelectionComplete,
}: UsePdfCharacterSelectionInput): void {
  const startCallbackRef = useRef(onSelectionStart);
  const completeCallbackRef = useRef(onSelectionComplete);
  const painter = useMemo(() => new PdfLiveSelectionPainter(), []);
  useLayoutEffect(() => { painter.setHosts(pageHosts); }, [painter, pageHosts]);
  startCallbackRef.current = onSelectionStart;
  completeCallbackRef.current = onSelectionComplete;

  useEffect(() => {
    const viewer = viewerRef.current;
    if (!enabled || viewer === null) return;
    const controller = new PdfCharacterSelectionController({
      viewer,
      doubleClickAction,
      pageStore,
      onSelectionStart: () => startCallbackRef.current(),
      onSelectionChange: selection => painter.render(selection),
      onSelectionComplete: (selection) => completeCallbackRef.current(selection),
    });
    return () => { controller.destroy(); painter.clear(); };
  }, [enabled, pageStore, viewerRef, doubleClickAction, version, painter]);
}
