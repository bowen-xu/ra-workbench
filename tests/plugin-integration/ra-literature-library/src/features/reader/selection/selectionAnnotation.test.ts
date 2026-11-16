import { expect, it, vi } from "vitest";
import { selectionAnnotation } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/selectionAnnotation";
import type { PdfPendingSelection } from "../../../../../../../../plugins/ra-literature-library/src/features/knowledge/PdfSelectionActions";
import { captureEvidenceImage } from "../../../../../../../apps/web/src/core/features/agent/evidenceImageGateway";
vi.mock("../../../../../../../apps/web/src/core/features/agent/evidenceImageGateway",()=>({captureEvidenceImage:vi.fn(async(bytes:readonly number[],mediaType:string)=>({mediaType,assetId:"a".repeat(64),captureId:"00000000-0000-4000-8000-000000000001",byteSize:bytes.length}))}));
it("captures text as an empty-comment annotation without rendering or importing a screenshot", async () => {
  vi.clearAllMocks();
  const rect = {x: .1, y: .2, width: .3, height: .4};
  const location = {startLine: 5, startColumn: 8, endLine: 10, endColumn: 22};
  const selection: PdfPendingSelection = {text: "原文 🧪", pageIndex: 4, pageLabel: "444", prefix: "", suffix: "", rects: [rect],
    segments: [{text: "原文 🧪", pageIndex: 4, pageLabel: "444", prefix: "", suffix: "", rects: [rect], location}], popupPageIndex: 4, popupRect: rect};
  const renderPageRegionPng = vi.fn(async () => {throw new Error("must not render text capture");});
  const [annotation] = await selectionAnnotation("paper", "pdf", "paper.pdf", selection, {renderPageRegionPng});
  expect(annotation).toEqual({type: "annotation", comment: "", source: "source://library/paper/pdf:#line{5:5-8:10-22}", title: "paper.pdf · 第 5 页", text: "原文 🧪"});
  expect(renderPageRegionPng).not.toHaveBeenCalled();
  expect(captureEvidenceImage).not.toHaveBeenCalled();
});
it("preserves a geometry-only PDF image snapshot without inventing text coordinates", async () => {
  const rect = {x:.1,y:.2,width:.3,height:.4};
  const selection: PdfPendingSelection = {text:"",pageIndex:2,pageLabel:"iii",prefix:"",suffix:"",segments:[],rects:[rect],popupPageIndex:2,popupRect:rect};
  const bytes = [137,80,78,71,13,10,26,10,1];
  const renderPageRegionPng = vi.fn(async () => "data:image/png;base64," + btoa(String.fromCharCode(...bytes)));
  const evidence = await selectionAnnotation("paper","pdf","figure.pdf",selection,{renderPageRegionPng});
  expect(renderPageRegionPng).toHaveBeenCalledWith(2,expect.objectContaining({x:.1,y:.2}));
  expect(evidence[0]?.source).toBe("source://library/paper/pdf");
  expect(evidence[0]?.text).toContain('"pageNumber":3');
  expect(captureEvidenceImage).toHaveBeenCalledWith(bytes,"image/png","figure.pdf");
  expect(evidence[0]?.type === "annotation" && evidence[0].image?.byteSize).toBe(bytes.length);
});
