// @vitest-environment happy-dom
import { expect,it,vi } from "vitest";
import { PdfViewAdapter } from "./PdfViewAdapter";

const fixture=vi.hoisted(()=>({getDocument:vi.fn(),setDocument:vi.fn(),setPageLabels:vi.fn()}));
vi.mock("pdfjs-dist/legacy/build/pdf.mjs",async importOriginal=>({...await importOriginal<object>(),getDocument:fixture.getDocument}));
vi.mock("pdfjs-dist/legacy/web/pdf_viewer.mjs",async importOriginal=>({...await importOriginal<object>(),PDFViewer:class {
  setDocument=fixture.setDocument;
  setPageLabels=fixture.setPageLabels;
}}));
it("starts the viewer and publishes thumbnail count while page labels and outline are still pending",async()=>{
  let resolveLabels!:(value:string[])=>void;
  const labels=new Promise<string[]>(resolve=>{resolveLabels=resolve;});
  const document={numPages:15,getPageLabels:vi.fn(()=>labels),getOutline:vi.fn(()=>new Promise(()=>undefined))};
  fixture.getDocument.mockReturnValue({promise:Promise.resolve(document),destroy:vi.fn().mockResolvedValue(undefined)});
  const callbacks={onDocumentReady:vi.fn(),onPageLabelsChange:vi.fn(),onReady:vi.fn(),onPageChange:vi.fn(),onScaleChange:vi.fn(),onViewAreaChange:vi.fn(),onPageHostsChange:vi.fn(),onFindMatchesChange:vi.fn(),onOutlineChange:vi.fn(),onCharacterSelectionAvailabilityChange:vi.fn(),onResourceChange:vi.fn(),onStatus:vi.fn()};
  const adapter=new PdfViewAdapter();
  await adapter.open({bytes:new Uint8Array(),container:window.document.createElement("div"),viewerElement:window.document.createElement("div"),callbacks});
  expect(callbacks.onDocumentReady).toHaveBeenCalledWith(15);
  expect(fixture.setDocument).toHaveBeenCalledWith(document);expect(callbacks.onReady).not.toHaveBeenCalled();
  expect(document.getOutline).toHaveBeenCalledOnce();expect(document.getPageLabels).toHaveBeenCalledOnce();
  resolveLabels(["i","1"]);await vi.waitFor(()=>expect(callbacks.onPageLabelsChange).toHaveBeenCalledWith(["i","1"]));
  expect(fixture.setPageLabels).toHaveBeenCalledWith(["i","1"]);adapter.close();
});
