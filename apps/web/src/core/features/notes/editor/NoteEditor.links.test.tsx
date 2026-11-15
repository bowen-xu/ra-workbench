// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { NoteEditor } from "./NoteEditor";
import { SOURCE_NAVIGATION_EVENT, type SourceNavigationRequest } from "../../agent/sourceNavigation";

vi.mock("../NotesClient", async () => {
  const { NoteCommandHistory } = await import("./NoteCommandHistory");
  const history = new NoteCommandHistory();
  return { useNotesClient: () => ({ gateway: {}, history }), useNotesVersion: () => 0 };
});
vi.mock("../../../services/notifications", () => ({ notify: { error: vi.fn() } }));
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it.each([false, true])("retains source anchors and navigates internally in readonly=%s notes", async isReadOnly => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const href = "source://library/paper/pdf:#line{3:12-4:14-20}";
  const page: NoteDocumentV1 = { schemaVersion: 1, pageId: notePageId("note"), blocks: [{
    id: noteBlockId("block"), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [
      { type: "link", href, content: [{ type: "text", text: "Paper", styles: ["bold"] }] },
      { type: "text", text: " ", styles: [] },
      { type: "link", href: "https://example.org", content: [{ type: "text", text: "Website", styles: [] }] },
    ] },
  }] };
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), changed = vi.fn(), opened = vi.spyOn(window, "open").mockReturnValue(null);
  cleanups.push(async () => { await act(async () => root.unmount()); host.remove(); });
  const requested = vi.fn((event: Event) => {
    event.preventDefault();
    (event as CustomEvent<SourceNavigationRequest>).detail.resolve();
  });
  window.addEventListener(SOURCE_NAVIGATION_EVENT, requested);
  cleanups.push(async () => window.removeEventListener(SOURCE_NAVIGATION_EVENT, requested));
  await act(async () => root.render(<NoteEditor document={page} onChange={changed} isReadOnly={isReadOnly} />));
  const anchor = host.querySelector<HTMLAnchorElement>("a")!;
  expect(anchor.getAttribute("href")).toBe(href);
  for (const type of ["click", "auxclick"]) {
    const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: type === "click" ? 0 : 1 });
    await act(async () => { (anchor.firstElementChild ?? anchor).dispatchEvent(event); });
    expect(event.defaultPrevented).toBe(true);
  }
  expect(requested).toHaveBeenCalledTimes(2);
  const request = (requested.mock.calls[0]![0] as CustomEvent<SourceNavigationRequest>).detail;
  expect(request.link.path).toEqual(["library", "paper", "pdf"]);
  expect(request.link.locator).toEqual({ kind: "line", pageNumber: 3, range: { startLine: 12, startColumn: 4, endLine: 14, endColumn: 20 } });
  expect(opened).not.toHaveBeenCalled();
  expect(changed).not.toHaveBeenCalled();
  expect(host.querySelectorAll("a")[1]?.getAttribute("href")).toBe("https://example.org");
});
