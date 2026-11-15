// @vitest-environment happy-dom
import { afterEach,expect,it,vi } from "vitest";
import type { PDFDocumentProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import { extractPdfPageTexts,loadPdfNavigationMetadata } from "./pdfDocumentRendering";

vi.mock("./pdfTextContent",()=>({readPdfTextContent:vi.fn(async()=>({items:[{str:"page text"}]}))}));
afterEach(()=>vi.useRealTimers());
function deferred<T>(){let resolve!:(value:T)=>void;let reject!:(reason:unknown)=>void;const promise=new Promise<T>((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};}
it("loads labels and outline independently and drops metadata after a document switch",async()=>{
  const labels=deferred<string[]|null>(),outline=deferred<null>();
  const document={getPageLabels:vi.fn(()=>labels.promise),getOutline:vi.fn(()=>outline.promise)} as unknown as PDFDocumentProxy;
  const callbacks={labels:vi.fn(),outline:vi.fn(),status:vi.fn()};let current=true;
  loadPdfNavigationMetadata(document,()=>current,callbacks);
  expect(document.getPageLabels).toHaveBeenCalledOnce();expect(document.getOutline).toHaveBeenCalledOnce();
  outline.resolve(null);await vi.waitFor(()=>expect(callbacks.outline).toHaveBeenCalledWith([]));
  expect(callbacks.labels).not.toHaveBeenCalled();current=false;
  labels.resolve(["i"]);await Promise.resolve();expect(callbacks.labels).not.toHaveBeenCalled();
});
it("keeps labels usable when outline fails and reports the local error",async()=>{
  const document={getPageLabels:vi.fn().mockResolvedValue(["i"]),getOutline:vi.fn().mockRejectedValue(new Error("bad outline"))} as unknown as PDFDocumentProxy;
  const callbacks={labels:vi.fn(),outline:vi.fn(),status:vi.fn()};
  loadPdfNavigationMetadata(document,()=>true,callbacks);
  await vi.waitFor(()=>expect(callbacks.status).toHaveBeenCalledWith(expect.stringContaining("bad outline")));
  expect(callbacks.labels).toHaveBeenCalledWith(["i"]);
});
it("yields between indexed pages so interactive work can run and abort stops later page requests",async()=>{
  vi.useFakeTimers();
  const abort=new AbortController(),cleanup=vi.fn();
  const getPage=vi.fn(async()=>{window.setTimeout(()=>abort.abort(),0);return {cleanup};});
  const document={numPages:100,getPage} as unknown as PDFDocumentProxy;
  const result=extractPdfPageTexts(document,()=>true,abort.signal);
  expect(getPage).not.toHaveBeenCalled();await vi.runAllTimersAsync();
  expect(await result).toEqual([]);expect(getPage).toHaveBeenCalledTimes(1);expect(cleanup).toHaveBeenCalledOnce();
});
it("extracts all text without retaining page geometry and cleans up each page",async()=>{
  const cleanup=vi.fn(),getPage=vi.fn().mockResolvedValue({cleanup});
  const document={numPages:2,getPage} as unknown as PDFDocumentProxy;
  expect(await extractPdfPageTexts(document,()=>true)).toEqual([{pageIndex:0,text:"page text"},{pageIndex:1,text:"page text"}]);
  expect(cleanup).toHaveBeenCalledTimes(2);
});
