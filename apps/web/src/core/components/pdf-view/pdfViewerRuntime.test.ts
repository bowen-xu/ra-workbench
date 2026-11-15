/** @vitest-environment happy-dom */
import { expect, it, vi } from "vitest";
import type { PDFViewer } from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import { applyPdfLocation } from "./pdfViewerRuntime";
import { readingNoteLocation } from "../../../../../../plugins/literature_library/src/features/reader/readerWorkspaceModel";
import { normalizeReaderLocation } from "@research-assistant/core/reader";
import type { ReadingNoteSource } from "@research-assistant/core/notes";

it("centers the union of all note rectangles rather than the first rectangle", () => {
  const scrollPageIntoView = vi.fn();
  const viewer = { scrollPageIntoView, getPageView: () => ({ viewport: { width: 1000, height: 2000, convertToPdfPoint: (x:number,y:number)=>[x,2000-y] } }) } as unknown as PDFViewer;
  const container = { clientWidth:800,clientHeight:600,style:{},scrollTop:0 } as unknown as HTMLElement;
  applyPdfLocation(viewer,container,{min:.1,max:5},{alignment:"center",position:{segments:[{pageIndex:0,rects:[{x:.1,y:.2,width:.2,height:.02},{x:.6,y:.5,width:.2,height:.05}]}]}},0);
  expect(scrollPageIntoView).toHaveBeenCalledWith({pageNumber:1,destArray:[null,{name:"XYZ"},50,1550,null],ignoreDestinationZoom:true,allowNegativeOffset:true});
  scrollPageIntoView.mockClear();
  applyPdfLocation(viewer,container,{min:.1,max:5},{position:{segments:[{pageIndex:0,rects:[{x:.1,y:.2,width:.2,height:.02}]}]}},0);
  expect(scrollPageIntoView.mock.calls[0]?.[0].destArray).toEqual([null,{name:"XYZ"},100,1600,null]);
});
it("preserves explicit note centering through the normalized navigation contract",()=>{
  const source: ReadingNoteSource = {paperId:"paper",paperTitle:"Fixture",attachmentId:"pdf",state:"exact",pageIndex:2,contentSha256:"a".repeat(64),rects:[{x:.3,y:.7,width:.00001,height:.00001}],quoteExact:"",quotePrefix:"",quoteSuffix:""};
  const location=normalizeReaderLocation(readingNoteLocation(source));
  expect(location?.alignment).toBe("center");expect(location?.position?.segments[0]?.rects).toEqual(source.rects);
});

it("selects the restored page before calculating a fit preset for mixed page sizes", () => {
  let pageNumber = 2;
  const fittedPages: number[] = [];
  const viewer = { get currentPageNumber() { return pageNumber; }, set currentPageNumber(value: number) { pageNumber = value; },
    set currentScaleValue(_value: string) { fittedPages.push(pageNumber); }, scrollPageIntoView: vi.fn() } as unknown as PDFViewer;
  applyPdfLocation(viewer, undefined, {min: .1, max: 5}, {pageIndex: 0, scale: {kind: "page-fit"}, top: 394, left: -23}, 0);
  expect(fittedPages).toEqual([1]);
});
