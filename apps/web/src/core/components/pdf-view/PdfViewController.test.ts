// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { PdfViewController } from "./PdfViewController";
import type { PdfViewAdapter, PdfViewOpenInput } from "./PdfViewAdapter";

function setup() {
  const requests: PdfViewOpenInput[] = [];
  const reject: ((error: Error) => void)[] = [];
  const adapter = { open: vi.fn((input: PdfViewOpenInput) => { requests.push(input); return new Promise<void>((_resolve, fail) => reject.push(fail)); }), close: vi.fn(), setActive: vi.fn(), suspend: vi.fn(), focus: vi.fn() };
  const controller = new PdfViewController(adapter as unknown as PdfViewAdapter, 50);
  const open = (revision: string, key = "document") => controller.open({ source: { key, revision, bytes: new Uint8Array([1]) }, container: document.createElement("div"), viewerElement: document.createElement("div"), active: true });
  const ready = (index: number) => requests[index]!.callbacks.onReady({ pageIndex: 2, pageCount: 4, pageLabels: [], scale: 1.25, viewState: { pageIndex: 2, scale: { kind: "numeric", value: 1.25 }, top: 420, scrollMode: "vertical", spreadMode: "none" } });
  return { controller, adapter, requests, reject, open, ready };
}
afterEach(() => vi.useRealTimers());
it("publishes document count and late labels without waiting for the first page, ignoring stale document metadata",()=>{
  const f=setup();void f.open("one");
  f.requests[0]!.callbacks.onDocumentReady?.(40);
  expect(f.controller.getSnapshot()).toMatchObject({ready:false,loading:true,pageCount:40});
  f.ready(0);f.requests[0]!.callbacks.onPageLabelsChange?.(["i","ii","1","2"]);
  expect(f.controller.getSnapshot().pageLabels).toEqual(["i","ii","1","2"]);
  void f.open("two","other");
  f.requests[0]!.callbacks.onDocumentReady?.(99);f.requests[0]!.callbacks.onPageLabelsChange?.(["old"]);
  expect(f.controller.getSnapshot().pageCount).not.toBe(99);expect(f.controller.getSnapshot().pageLabels).not.toEqual(["old"]);
  f.controller.close();
});
it("stores every selection in one snapshot and advances the selection revision even on empty clears",()=>{
  const f=setup(), selection={text:"quote",segments:[],focusPageIndex:0,focusRect:{x:.1,y:.2,width:.3,height:.04}};
  f.controller.setSelection(selection);expect(f.controller.getSnapshot().selection).toBe(selection);
  const selectedRevision=f.controller.getSnapshot().selectionRevision;
  f.controller.clearSelection();expect(f.controller.getSnapshot().selection).toBeUndefined();
  const clearedRevision=f.controller.getSnapshot().selectionRevision;
  f.controller.clearSelection();expect(f.controller.getSnapshot().selectionRevision).toBeGreaterThan(clearedRevision);
  f.controller.close();expect(f.controller.getSnapshot().selectionRevision).toBeGreaterThan(selectedRevision);
});
it("retains the displayed version after replacement failure and continues receiving its viewport events", async () => {
  const f = setup(); void f.open("one"); f.ready(0);
  const replacement = f.open("two");
  expect(f.controller.getSnapshot()).toMatchObject({ ready: true, loading: true, source: { revision: "one" } });
  expect(f.requests[1]?.preserveView).toBe(true);
  expect(f.controller.isShowing({ key: "document", revision: "two" })).toBe(false);
  f.reject[1]!(new Error("corrupt PDF")); await replacement;
  f.requests[0]!.callbacks.onPageChange(3);
  expect(f.controller.getSnapshot()).toMatchObject({ source: { revision: "one" }, loading: false, pageIndex: 3, error: "corrupt PDF" });
  f.controller.close();
});
it("rejects stale ready, search and selection availability events after a document switch", () => {
  const f = setup(); void f.open("one"); f.ready(0); void f.open("two", "other"); f.ready(1);
  f.ready(0); f.requests[0]!.callbacks.onFindMatchesChange(2, 9); f.requests[0]!.callbacks.onCharacterSelectionAvailabilityChange(true);
  expect(f.controller.getSnapshot()).toMatchObject({ source: { key: "other", revision: "two" }, findTotal: 0, characterSelectionAvailable: false });
  expect(f.requests[1]?.preserveView).toBe(false);
  f.controller.close();
});
it("isolates two viewers and cancels background suspension on resume and close", () => {
  vi.useFakeTimers(); const a = setup(), b = setup(); void a.open("one"); a.ready(0); void b.open("one"); b.ready(0);
  a.requests[0]!.callbacks.onPageChange(1); expect(b.controller.getSnapshot().pageIndex).toBe(2);
  a.controller.setActive(false); vi.advanceTimersByTime(30); a.controller.setActive(true); vi.advanceTimersByTime(50);
  expect(a.adapter.suspend).not.toHaveBeenCalled();
  a.controller.setActive(false); vi.advanceTimersByTime(50); expect(a.adapter.suspend).toHaveBeenCalledOnce();
  expect(b.adapter.suspend).not.toHaveBeenCalled();
  a.controller.setActive(false); a.controller.close(); vi.advanceTimersByTime(100); expect(a.adapter.suspend).toHaveBeenCalledOnce(); b.controller.close();
});
