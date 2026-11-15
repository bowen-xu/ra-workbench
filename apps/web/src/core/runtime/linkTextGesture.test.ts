// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {installBrowserLinkRouting} from "./browserLinkRouting";
import type {AppRuntime} from "./appRuntime";

let dispose: (() => void) | undefined;
afterEach(() => {dispose?.(); dispose = undefined; document.getSelection()?.removeAllRanges(); document.body.innerHTML = ""; vi.restoreAllMocks();});
function fixture(href = "https://example.org/link") {
  const open = vi.fn(async (_url: string) => {});
  const runtime: AppRuntime = {mode: "desktop", storage: localStorage, gateways: {}, browser: {open}, dispose() {}};
  dispose = installBrowserLinkRouting(document, runtime);
  document.body.innerHTML = `<div class="bn-editor" contenteditable="true"><a data-inline-content-type="link" href="${href}"><span>Selectable link</span></a></div>`;
  const anchor = document.querySelector("a")!, span = anchor.querySelector("span")!, owner = vi.fn();
  anchor.addEventListener("click", owner);
  const pointer = (type: string, x = 0, id = 1) => span.dispatchEvent(new PointerEvent(type, {
    bubbles: true, cancelable: true, pointerId: id, pointerType: "mouse", isPrimary: true, button: 0, clientX: x, clientY: 0,
  }));
  const click = (detail = 1) => {
    const event = new MouseEvent("click", {bubbles: true, cancelable: true, button: 0, detail});
    span.dispatchEvent(event); return event;
  };
  const select = () => {
    const range = document.createRange(); range.setStart(span.firstChild!, 2); range.setEnd(span.firstChild!, 8);
    const selection = document.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
  };
  return {open, owner, pointer, click, select};
}

it.each(["https://example.org/link", "source://notes/page"])("keeps native selection after dragging a nested link without invoking navigation (%s)", href => {
  const f = fixture(href);
  f.pointer("pointerdown"); f.pointer("pointermove", 12); f.select(); f.pointer("pointerup", 12);
  expect(f.click().defaultPrevented).toBe(true);
  expect(document.getSelection()?.toString()).toBe("lectab");
  expect(f.open).not.toHaveBeenCalled(); expect(f.owner).not.toHaveBeenCalled();
  // The completed selection must not poison a new deliberate click.
  f.pointer("pointerdown", 12); f.pointer("pointerup", 12); f.click();
  if (href.startsWith("https:")) expect(f.open).toHaveBeenCalledExactlyOnceWith(href);
  else expect(f.owner).toHaveBeenCalledOnce();
});

it("uses selection change as well as distance and observes pointerup without an intermediate move", () => {
  const f = fixture();
  f.pointer("pointerdown"); f.select(); f.pointer("pointerup", 1);
  f.click(); expect(f.open).not.toHaveBeenCalled();
  document.getSelection()?.removeAllRanges();
  f.pointer("pointerdown"); f.pointer("pointerup", 10);
  f.click(); expect(f.open).not.toHaveBeenCalled();
});

it("leaves keyboard and programmatic activation independent of the last pointer gesture", () => {
  const f = fixture(); f.pointer("pointerdown"); f.pointer("pointerup", 10);
  f.click(0); expect(f.open).toHaveBeenCalledOnce();
});

it.each(["pointercancel", "blur", "unmount"])("clears cancelled gesture state on %s", reason => {
  const f = fixture(); f.pointer("pointerdown"); f.pointer("pointermove", 10);
  const editor = document.querySelector(".bn-editor")!;
  expect(editor.hasAttribute("data-link-text-selecting")).toBe(true);
  if (reason === "pointercancel") f.pointer("pointercancel", 10);
  else if (reason === "blur") window.dispatchEvent(new Event("blur"));
  else {dispose?.(); dispose = undefined;}
  expect(editor.hasAttribute("data-link-text-selecting")).toBe(false);
  f.pointer("pointerup", 10); f.click();
  if (reason === "unmount") expect(f.owner).toHaveBeenCalledOnce();
  else expect(f.open).toHaveBeenCalledOnce();
});

it("changes the cursor only for active selection, then restores it without discarding the range", () => {
  const f = fixture(), editor = document.querySelector(".bn-editor")!;
  f.pointer("pointerdown"); f.pointer("pointermove", 1);
  expect(editor.hasAttribute("data-link-text-selecting")).toBe(false);
  f.select(); document.dispatchEvent(new Event("selectionchange"));
  expect(editor.hasAttribute("data-link-text-selecting")).toBe(true);
  f.pointer("pointerup", 1);
  expect(editor.hasAttribute("data-link-text-selecting")).toBe(false);
  expect(document.getSelection()?.toString()).toBe("lectab");
  expect(f.click().defaultPrevented).toBe(true);
});

it("does not classify a different pointer or an ordinary anchor as an editor text drag", () => {
  const f = fixture(); f.pointer("pointerdown"); f.pointer("pointermove", 20, 2); f.pointer("pointerup");
  f.click(); expect(f.open).toHaveBeenCalledOnce();
  document.querySelector("a")!.removeAttribute("data-inline-content-type");
  f.pointer("pointerdown"); f.pointer("pointerup", 20); f.click();
  expect(f.open).toHaveBeenCalledTimes(2);
});
