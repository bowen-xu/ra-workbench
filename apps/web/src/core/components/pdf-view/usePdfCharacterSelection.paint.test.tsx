// @vitest-environment happy-dom
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { usePdfCharacterSelection } from "./usePdfCharacterSelection";
import type { PdfCharacterSelectionControllerOptions } from "./pdfCharacterSelectionController";
import { PdfPageCharacterStore } from "./pdfPageCharacterModel";

const fixture = vi.hoisted(() => ({ options: undefined as PdfCharacterSelectionControllerOptions | undefined }));
vi.mock("./pdfCharacterSelectionController", () => ({ PdfCharacterSelectionController: class {
  constructor(options: PdfCharacterSelectionControllerOptions) { fixture.options = options; }
  destroy() { fixture.options?.onSelectionChange(undefined); }
} }));

it("paints live rectangles synchronously without scheduling a React render, and rebinds/cleans page hosts", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"), page = document.createElement("div"), replacement = document.createElement("div");
  document.body.append(host, page, replacement);
  const root = createRoot(host), store = new PdfPageCharacterStore(); let renders = 0;
  function Harness({ target = page, enabled = true }: { target?: HTMLElement; enabled?: boolean }) {
    renders++;
    const viewerRef = useRef<HTMLDivElement>(null);
    usePdfCharacterSelection({ enabled, viewerRef, pageStore: store, pageHosts: [{ pageIndex: 0, element: target }],
      onSelectionStart() {}, onSelectionComplete() {} });
    return <div ref={viewerRef} />;
  }
  try {
    await act(async () => root.render(<Harness />)); const initial = renders;
    // Intentionally outside React act: the DOM must change before React can commit anything.
    fixture.options?.onSelectionChange({ segments: [{ pageIndex: 0, rects: [{ x: .1, y: .2, width: .3, height: .04 }] }] });
    expect(page.querySelector<HTMLElement>(".pending-selection-highlight")?.style.left).toBe("10%");
    fixture.options?.onSelectionChange({ segments: [{ pageIndex: 0, rects: [{ x: .2, y: .2, width: .4, height: .04 }] }] });
    expect(page.querySelector<HTMLElement>(".pending-selection-highlight")?.style.width).toBe("40%");
    expect(renders).toBe(initial);
    await act(async () => root.render(<Harness target={replacement} />));
    expect(page.querySelector(".pdf-live-selection-layer")).toBeNull();
    expect(replacement.querySelector<HTMLElement>(".pending-selection-highlight")?.style.width).toBe("40%");
    await act(async () => root.render(<Harness target={replacement} enabled={false} />));
    expect(replacement.querySelector(".pdf-live-selection-layer")).toBeNull();
  } finally {
    await act(async () => root.unmount()); host.remove(); page.remove(); replacement.remove(); vi.unstubAllGlobals();
  }
});
