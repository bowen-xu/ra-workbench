// @vitest-environment happy-dom
import { act, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DragDropEvent } from "@tauri-apps/api/webview";
import type { EventCallback } from "@tauri-apps/api/event";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import { emptyLiteratureDraft, type LiteratureItem } from "@research-assistant/core/literature";
import type { LiteratureGateway } from "../../../../../../plugins/ra-literature-library/src/public/literatureGateway";
import { useLiteratureFileDrop } from "../../../../../../plugins/ra-literature-library/src/components/useLiteratureFileDrop";
import { literatureFileDropLabel } from "../../../../../../plugins/ra-literature-library/src/components/literatureFileDrop";

const native = vi.hoisted(() => ({
  handler: undefined as EventCallback<DragDropEvent> | undefined,
  stop: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({
  onDragDropEvent: vi.fn(async (handler: EventCallback<DragDropEvent>) => {
    native.handler = handler; return native.stop;
  }),
}) }));
vi.mock("../../../../../apps/web/src/core/runtime/platformCore", () => ({ isDesktopRuntime: () => true }));

function item(id: string): LiteratureItem {
  return { ...emptyLiteratureDraft(), id, title: id, attachments: [], metadataSources: [],
    projectIds: [], readingStatus: "unread", createdAt: "", updatedAt: "" };
}
const items = [item("first"), item("second")];
let host: HTMLDivElement;
let root: Root;
const attachPdf = vi.fn(async (_id: string, _path: string) => ({}));
const review = vi.fn();
const onAttached = vi.fn(async () => undefined);
const onError = vi.fn();
const onWarning = vi.fn();
const gateway = { attachPdf } as unknown as LiteratureGateway;

function Fixture({ selected, enabled = true, isTrash = false }: {
  selected?: LiteratureItem | undefined; enabled?: boolean; isTrash?: boolean;
}) {
  const ref = useRef<HTMLElement>(null);
  const drop = useLiteratureFileDrop({ root: ref, items, selected, enabled, isTrash, gateway,
    reviewPdfPaths: review, onAttached, onError, onNotice: vi.fn(), onWarning });
  return <section ref={ref}>
    <header>Toolbar</header>
    <div data-literature-drop-zone="list">
      <div data-testid="columns">Header</div>
      {items.map(value => <div key={value.id} data-literature-drop-item={value.id}><button>{value.title}</button></div>)}
      <div data-testid="blank">Blank</div>
    </div>
    <aside data-literature-drop-zone="detail"><span>Detail</span></aside>
    <output>{drop.target ? literatureFileDropLabel(drop.target) : ""}</output>
  </section>;
}

async function render(props: Parameters<typeof Fixture>[0] = {}) {
  await act(async () => root.render(<Fixture selected={items[0]} {...props} />));
}
async function send(selector: string, type: "over" | "drop" = "drop", paths = ["/tmp/a.pdf"]) {
  const element = host.querySelector(selector);
  if (!element) throw new Error(`Missing target: ${selector}`);
  const hit = vi.spyOn(document, "elementFromPoint").mockReturnValue(element);
  await act(async () => {
    native.handler?.({ event: "tauri://drag-drop", id: 1, payload: type === "drop"
      ? { type, paths, position: new PhysicalPosition(200, 100) }
      : { type, position: new PhysicalPosition(200, 100) } });
  });
  expect(hit).toHaveBeenCalledWith(100, 50);
  hit.mockRestore();
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("devicePixelRatio", 2);
  vi.stubGlobal("navigator", { platform: "Win32" });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

describe("native literature file drop routing", () => {
  it("does not halve AppKit coordinates on Retina and mistake the detail pane for a row", async () => {
    vi.stubGlobal("navigator", { platform: "MacIntel" });
    await render({ selected: undefined });
    const hit = vi.spyOn(document, "elementFromPoint").mockImplementation((x, y) =>
      x >= 160 && y >= 80 ? host.querySelector("aside") : host.querySelector('[data-literature-drop-item="first"]'));
    await act(async () => {
      for (const type of ["over", "drop"] as const) native.handler?.({ event: "drop", id: 1,
        payload: type === "over" ? { type, position: new PhysicalPosition(200, 100) }
          : { type, paths: ["/tmp/a.pdf"], position: new PhysicalPosition(200, 100) } });
    });
    expect(hit).toHaveBeenCalledWith(200, 100);
    expect(review).toHaveBeenCalledWith(["/tmp/a.pdf"]);
    expect(attachPdf).not.toHaveBeenCalled();
    hit.mockRestore();
  });
  it("attaches to the hovered row regardless of selection and does not create an item", async () => {
    await render();
    await send('[data-literature-drop-item="second"] button', "over");
    expect(host.querySelector("output")?.textContent).toContain("「second」的附件");
    await send('[data-literature-drop-item="second"] button');
    expect(attachPdf).toHaveBeenCalledWith("second", "/tmp/a.pdf");
    expect(onAttached).toHaveBeenCalledWith("second"); expect(review).not.toHaveBeenCalled();
    expect(host.querySelector("output")?.textContent).toBe("");
  });

  it("uses the current selected item for detail drops without resubscribing", async () => {
    await render(); await render({ selected: items[1] });
    await send("aside span");
    expect(attachPdf).toHaveBeenCalledWith("second", "/tmp/a.pdf");
    expect(native.stop).not.toHaveBeenCalled();
  });

  it.each(['[data-testid="columns"]', '[data-testid="blank"]'])("reviews standalone PDFs in %s even with selection", async selector => {
    await render(); await send(selector);
    expect(review).toHaveBeenCalledWith(["/tmp/a.pdf"]); expect(attachPdf).not.toHaveBeenCalled();
  });

  it("reviews a standalone PDF when the detail pane has no selection", async () => {
    await render({ selected: undefined });
    await send("aside");
    expect(review).toHaveBeenCalledWith(["/tmp/a.pdf"]); expect(attachPdf).not.toHaveBeenCalled();
  });

  it("ignores toolbar and portaled/foreign targets and stops listening while inactive", async () => {
    await render(); await send("header");
    const foreign = document.createElement("div"); foreign.dataset.literatureDropZone = "detail";
    const hit = vi.spyOn(document, "elementFromPoint").mockReturnValue(foreign);
    await act(async () => native.handler?.({ event: "drop", id: 1, payload: { type: "drop", paths: ["/tmp/a.pdf"], position: new PhysicalPosition(0, 0) } }));
    hit.mockRestore();
    await render({ enabled: false });
    await act(async () => native.handler?.({ event: "drop", id: 1, payload: { type: "drop", paths: ["/tmp/a.pdf"], position: new PhysicalPosition(0, 0) } }));
    expect(native.stop).toHaveBeenCalledOnce(); expect(attachPdf).not.toHaveBeenCalled(); expect(review).not.toHaveBeenCalled();
  });

  it("blocks trash and ignores non-PDF files", async () => {
    await render({ isTrash: true }); await send("aside");
    expect(onWarning).toHaveBeenCalledWith(expect.stringContaining("恢复"));
    await render(); await send("aside", "drop", ["/tmp/image.png"]);
    expect(attachPdf).not.toHaveBeenCalled(); expect(review).not.toHaveBeenCalled();
  });

  it("keeps a multi-file drop bound to its original item through async copies and reports partial errors", async () => {
    let finish: (() => void) | undefined;
    attachPdf.mockImplementationOnce(async () => { await new Promise<void>(resolve => { finish = resolve; }); return {}; });
    attachPdf.mockRejectedValueOnce(new Error("E_PDF: invalid file"));
    await render(); await send("aside", "drop", ["/tmp/a.pdf", "/tmp/b.pdf", "/tmp/c.PDF", "/tmp/a.pdf", "/tmp/n.txt"]);
    await render({ selected: items[1] });
    await act(async () => { finish?.(); });
    expect(attachPdf.mock.calls).toEqual([["first", "/tmp/a.pdf"], ["first", "/tmp/b.pdf"], ["first", "/tmp/c.PDF"]]);
    expect(onAttached).toHaveBeenCalledWith("first");
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("b.pdf: E_PDF: invalid file"));
    expect(review).not.toHaveBeenCalled();
  });
});
