import { expect, it, vi } from "vitest";
import type { DoclingParseRunDetail, DoclingPageOverlay } from "../../../../../../../apps/web/src/core/services/doclingGateway";
import type { PdfPendingSelection } from "../../../../../../../../plugins/ra-literature-library/src/features/knowledge/PdfSelectionActions";
import { selectionDoclingContext } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/selectionDoclingContext";
import { selectionExplanationMessage, prepareSelectionExplanationMessage } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/selectionExplanationMessage";

const attachment = { id: "pdf", contentSha256: "a".repeat(64), displayName: "paper.pdf" };
const item = { id: "paper", title: "Synthetic paper" };
const rect = { x: .1, y: .2, width: .3, height: .1 };
const selection: PdfPendingSelection = { text: "Selected quote", pageIndex: 0, pageLabel: "440", rects: [rect], prefix: "", suffix: "",
  segments: [{ pageIndex: 0, pageLabel: "440", rects: [rect], text: "Selected quote", prefix: "", suffix: "" }], popupPageIndex: 0, popupRect: rect };
const detail: DoclingParseRunDetail = { id: "parse", itemId: item.id, attachmentId: attachment.id, contentSha256: attachment.contentSha256,
  serviceBaseUrl: "http://local.fixture", doclingStatus: "success", parseStatus: "success", createdAt: "2026-10-12T01:00:00Z", finishedAt: "2026-10-12T01:00:01Z", rawResponse: {},
  jsonContent: { texts: [{ self_ref: "#/texts/0", label: "paragraph", text: "Parsed paragraph", prov: [{ page_no: 1 }] }, { text: "Unrelated private paragraph" }],
    tables: [{ label: "table", data: { table_cells: [{ text: "Cell A", row_span: 2 }] } }],
    pictures: [{ label: "picture", captions: [{ $ref: "#/texts/0" }], image: { uri: "file:///private/image.png" } }] } };
function gateway(run = detail) {
  const overlay: DoclingPageOverlay = { attachmentId: attachment.id, parseRunId: run.id, pageIndex: 0, items: [
    { id: "text", kind: "paragraph", rawRef: "#/texts/0", regions: [{ pageIndex: 0, rect }] },
    { id: "table", kind: "table", rawRef: "#/tables/0", regions: [{ pageIndex: 0, rect }] },
    { id: "picture", kind: "picture", rawRef: "#/pictures/0", regions: [{ pageIndex: 0, rect }] },
    { id: "unrelated", kind: "paragraph", rawRef: "#/texts/1", regions: [{ pageIndex: 0, rect: { ...rect, y: .8 } }] },
  ] };
  return { listParseRuns: vi.fn(async () => [run]), getParseRun: vi.fn(async () => run), getPageOverlay: vi.fn(async () => overlay) };
}

it("includes only matching Docling blocks, preserving table cells and picture positions separately from PDF quotes", async () => {
  const api = gateway();
  const result = await selectionDoclingContext(api, item, attachment, selection);
  expect(result).toMatchObject({ state: "available", parseRunId: "parse", blocks: [
    { ref: "#/texts/0", content: { text: "Parsed paragraph", prov: [{ page_no: 1 }] } },
    { ref: "#/tables/0", content: { data: { table_cells: [{ text: "Cell A", row_span: 2 }] } } },
    { ref: "#/pictures/0", regions: [{ pageIndex: 0, rect }] },
  ] });
  const message = selectionExplanationMessage(item, attachment, selection, result);
  expect(message).toContain("【来源】Synthetic paper");
  expect(message).toContain("原文：Selected quote");
  expect(message).toContain("文献 ID：paper");
  expect(message).toContain("文件：paper.pdf");
  expect(message).toContain("文件 ID：pdf");
  expect(message).not.toContain("PDF：");
  expect(message).not.toContain("Docling：");
  expect(message).not.toContain(".docling-cache");
  expect(message).not.toContain("preview://");
  expect(message).toContain("通过来源搜索／读取工具查阅文件中的相关上下文和术语定义");
  expect(message).not.toContain("Docling");
  expect(message).not.toContain("PDF");
  expect(message).not.toContain("解析 parse");
  expect(message).toContain("【定位引用】#/texts/0、#/tables/0、#/pictures/0");
  expect(message).not.toContain("Parsed paragraph");
  expect(message).not.toContain(attachment.contentSha256);
  expect(message).not.toContain("Unrelated private paragraph");
  expect(message).not.toContain("file:///private/image.png");
  expect(message).not.toContain(detail.serviceBaseUrl);
  expect(api.getPageOverlay).toHaveBeenCalledExactlyOnceWith("pdf", 0, "parse");
});

it("does not use an older attachment version or fetch its parse data", async () => {
  const api = gateway({ ...detail, contentSha256: "b".repeat(64) });
  expect(await selectionDoclingContext(api, item, attachment, selection)).toEqual({ state: "unavailable", reason: "notParsed" });
  expect(api.getParseRun).not.toHaveBeenCalled(); expect(api.getPageOverlay).not.toHaveBeenCalled();
});

it("rejects a mismatched overlay and distinguishes missing geometry from read errors", async () => {
  const api = gateway(); const overlay = await api.getPageOverlay();
  api.getPageOverlay.mockResolvedValueOnce({ ...overlay, parseRunId: "other-parse" });
  expect(await selectionDoclingContext(api, item, attachment, selection)).toEqual({ state: "unavailable", reason: "readFailed" });
  api.getPageOverlay.mockResolvedValueOnce({ ...overlay, items: [] });
  expect(await selectionDoclingContext(api, item, attachment, selection)).toEqual({ state: "unavailable", reason: "noMatchingBlocks" });
  api.listParseRuns.mockRejectedValueOnce(new Error("local failure"));
  expect(await selectionDoclingContext(api, item, attachment, selection)).toEqual({ state: "unavailable", reason: "readFailed" });
});

it("reports oversized structured material without silently truncating table data", async () => {
  const api = gateway({ ...detail, jsonContent: { texts: [{ text: "x".repeat(33 * 1024) }], tables: [{}], pictures: [{}] } });
  expect(await selectionDoclingContext(api, item, attachment, selection)).toEqual({ state: "unavailable", reason: "budget" });
});

it("emits exact one-based inclusive page locations and blocks sending without Docling or precise positions", async () => {
  const api = gateway();
  const controller = { resolveQuoteSelection: vi.fn(async () => undefined) };
  const positioned = { ...selection, segments: [{ ...selection.segments[0]!, location: { startLine: 12, startColumn: 4, endLine: 14, endColumn: 26 } }] };
  const message = await prepareSelectionExplanationMessage(item, attachment, positioned, api, controller);
  expect(message).toContain("第 1 页（印刷 440）：12:4–14:26");
  expect(controller.resolveQuoteSelection).not.toHaveBeenCalled();
  await expect(prepareSelectionExplanationMessage(item, attachment, selection, api, controller)).rejects.toThrow("起止行列");
  expect(controller.resolveQuoteSelection).toHaveBeenCalledWith(0, "Selected quote", "", "");
  api.listParseRuns.mockResolvedValueOnce([]);
  await expect(prepareSelectionExplanationMessage(item, attachment, positioned, api, controller)).rejects.toThrow("请先解析 PDF");
});

it("validates the stored parse identity against the selected literature item and PDF version", async () => {
  for (const override of [{ itemId: "other-paper" }, { attachmentId: "other-pdf" }, { contentSha256: "b".repeat(64) }, { id: "other-parse" }]) {
    const api = gateway();
    api.getParseRun.mockResolvedValueOnce({ ...detail, ...override });
    expect(await selectionDoclingContext(api, item, attachment, selection)).toEqual({ state: "unavailable", reason: "readFailed" });
  }
});
