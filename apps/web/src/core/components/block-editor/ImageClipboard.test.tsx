// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {NodeSelection, TextSelection} from "@tiptap/pm/state";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {BlockSetSelection} from "./BlockSetSelection";
import {noteEditorSchema, type NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {WritingNoteOwner, writingNoteSchema} from "../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteSchema";
import {notify} from "../../services/notifications";
import {ManagedAssetIngress} from "./ManagedAssetIngress";
import {embeddedImageClipboardFile} from "./ImageClipboard";
import {NoteClipboard} from "../../features/notes/editor/NoteClipboard";
import {blockEditorDefaults} from "./defaults";

const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jI1sAAAAASUVORK5CYII="), c => c.charCodeAt(0));
const client = vi.hoisted(() => ({gateway: {readNoteAsset: vi.fn()}}));
vi.mock("../../features/notes/NotesClient", () => ({useNotesClient: () => client, useNotesVersion: () => 0}));
vi.mock("../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteAssets", () => ({readWritingNoteAsset: async () => png, exportWritingNoteAsset: vi.fn()}));
vi.mock("../../services/notifications", () => ({notify: {error: vi.fn()}}));
const cleanup: (() => Promise<void>)[] = [];
const write = vi.fn(async (_items: readonly ClipboardItem[]) => {}), drawImage = vi.fn();
let finish: BlobCallback, nativeCopyWrites = 0;
class ClipboardFixture {
  readonly types: string[];
  constructor(private readonly data: Record<string, Blob | Promise<Blob>>) {this.types = Object.keys(data);}
  getType(type: string) {return Promise.resolve(this.data[type]!);}
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.stubGlobal("ClipboardItem", ClipboardFixture);
  Object.defineProperty(navigator, "clipboard", {configurable: true, value: {write}});
  write.mockReset().mockResolvedValue(undefined); nativeCopyWrites = 0;
  drawImage.mockClear(); vi.mocked(notify.error).mockClear();
  client.gateway.readNoteAsset.mockResolvedValue(png);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({drawImage} as unknown as ReturnType<HTMLCanvasElement["getContext"]>);
  vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation(callback => {finish = callback;});
});
afterEach(async () => {for (const release of cleanup.splice(0)) await release(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
async function mount(kind: "note" | "writing", readOnly = false) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const create = BlockNoteEditor.create as unknown as (options: Partial<NoteEditorInstance["options"]>) => NoteEditorInstance;
  // Use the same native paste policy as both production editor constructors.
  const editor = create({...blockEditorDefaults, schema: kind === "note" ? noteEditorSchema : writingNoteSchema as unknown as typeof noteEditorSchema,
    initialContent: [
      {id: "picture", type: "managedImage", props: {assetId: "asset", mimeType: "image/png", previewWidth: 200, caption: "caption"}},
      {id: "text", type: "paragraph", content: "ordinary text"},
    ]});
  const onFiles = vi.fn();
  await act(async () => root.render(<WritingNoteOwner.Provider value="fixture-owner"><NoteEditorSurface editor={editor} onChange={() => {}} readOnly={readOnly}>{kind === "note" && <NoteClipboard editor={editor}/>}<ManagedAssetIngress editor={editor} onFiles={onFiles}/></NoteEditorSurface></WritingNoteOwner.Provider>));
  cleanup.push(async () => {await act(async () => root.unmount()); host.remove();});
  await vi.waitFor(() => expect(host.querySelector(".note-image-block img")).not.toBeNull());
  const image = host.querySelector<HTMLImageElement>(".note-image-block img")!;
  Object.defineProperties(image, {complete: {value: true}, naturalWidth: {value: 800}, naturalHeight: {value: 300}});
  const position = (id: string) => {let found = 0; editor.prosemirrorView.state.doc.descendants((node, pos) => {if (node.attrs.id === id) found = pos;}); return found;};
  const select = async (style: "node" | "block") => {await act(async () => {
    const view = editor.prosemirrorView;
    view.dispatch(view.state.tr.setSelection(style === "node" ? NodeSelection.create(view.state.doc, position("picture") + 1) : BlockSetSelection.create(view.state.doc, ["picture"])));
  });};
  const copy = (type = "copy", supportsCustomMime = true) => {
    const values = new Map<string, string>(), event = new Event(type, {bubbles: true, cancelable: true});
    Object.defineProperty(event, "clipboardData", {value: {getData: (mime: string) => values.get(mime) ?? "", setData: (mime: string, value: string) => {if (supportsCustomMime || mime !== "blocknote/html") values.set(mime, value);}, clearData: () => values.clear()}});
    act(() => {editor.domElement!.dispatchEvent(event);});
    // WebKit Editor::dispatchClipboardEvent commits canceled copy event data
    // only when the DataTransfer still has data, after DOM handlers return.
    if (event.defaultPrevented && values.size) nativeCopyWrites++;
    return values;
  };
  return {editor, host, image, position, select, copy, onFiles};
}
it.each(["note", "writing"] as const)("%s copies full-resolution PNG and portable rich HTML without changing the document", async kind => {
  const f = await mount(kind); await f.select("node");
  const before = JSON.stringify(f.editor.document);
  const values = f.copy();
  expect(write).toHaveBeenCalledOnce(); // Starts before asynchronous encoding.
  expect(drawImage).toHaveBeenCalledWith(f.image, 0, 0);
  const canvas = vi.mocked(HTMLCanvasElement.prototype.toBlob).mock.instances[0]! as HTMLCanvasElement;
  expect(canvas.width).toBe(800); expect(canvas.height).toBe(300);
  finish(new Blob([png], {type: "image/png"}));
  const item = write.mock.calls[0]![0][0]!;
  expect(item.types).toEqual(["image/png", "text/html", "text/plain"]);
  expect((await item.getType("image/png")).type).toBe("image/png");
  const html = await (await item.getType("text/html")).text();
  expect(html).toContain("data:image/png;base64,"); expect(html).toContain('data-asset-id="asset"');
  expect(values.size).toBe(0); // The composite async write owns all formats.
  expect(JSON.stringify(f.editor.document)).toBe(before);
  const selected = f.editor.prosemirrorView.state.selection as NodeSelection;
  expect(selected.node.type.name === "blockContainer" ? selected.node.attrs.id : selected.$from.parent.attrs.id).toBe("picture");
});
it("copies one explicitly selected image block, including from a read-only document", async () => {
  const f = await mount("note", true); await f.select("block"); f.copy();
  expect(write).toHaveBeenCalledOnce(); finish(new Blob([png], {type: "image/png"}));
  await write.mock.calls[0]![0][0]!.getType("text/html");
});

it("does not race the browser copy commit while promised image data is encoding", async () => {
  const f = await mount("note"); await f.select("node");
  write.mockImplementationOnce(async items => {
    const changeCountAtStart = nativeCopyWrites;
    const item = items[0]!;
    await Promise.all(item.types.map(type => item.getType(type)));
    // WebKit Clipboard::ItemWriter rejects a write if the system pasteboard
    // changed between starting it and collecting its promised representations.
    if (changeCountAtStart !== nativeCopyWrites) throw new DOMException("pasteboard changed", "NotAllowedError");
  });
  f.copy();
  finish(new Blob([png], {type:"image/png"}));
  await expect(write.mock.results[0]!.value).resolves.toBeUndefined();
  expect(nativeCopyWrites).toBe(0);
  expect(notify.error).not.toHaveBeenCalled();
});
it("leaves text, multiple blocks and cut on the native clipboard path", async () => {
  const f = await mount("note"), view = f.editor.prosemirrorView;
  await act(async () => view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, f.position("text") + 2, f.position("text") + 6))));
  f.copy(); expect(write).not.toHaveBeenCalled();
  await act(async () => view.dispatch(view.state.tr.setSelection(BlockSetSelection.create(view.state.doc, ["picture", "text"]))));
  f.copy(); expect(write).not.toHaveBeenCalled();
  await f.select("node"); await act(async () => {f.copy("cut");}); expect(write).not.toHaveBeenCalled();
});
it("reports rejected image writes without a second clipboard publication", async () => {
  const f = await mount("note"); await f.select("node");
  write.mockRejectedValueOnce(new DOMException("denied", "NotAllowedError"));
  const values = f.copy(); finish(new Blob([png], {type: "image/png"}));
  await vi.waitFor(() => expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("NOTE_IMAGE_COPY_FAILED")));
  expect(values.size).toBe(0); expect(nativeCopyWrites).toBe(0);
});

it("retains native rich copy when the image writer cannot start synchronously", async () => {
  const f = await mount("note"); await f.select("node");
  write.mockImplementationOnce(() => {throw new DOMException("cannot start", "NotAllowedError");});
  const values = f.copy(); finish(new Blob([png], {type:"image/png"}));
  expect(values.get("blocknote/html")).toBeTruthy(); expect(nativeCopyWrites).toBe(1);
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("NOTE_IMAGE_COPY_FAILED"));
});

it.each(["note", "writing"] as const)("%s pastes the composite clipboard as a rich image block instead of importing its PNG twice", async kind => {
  const f = await mount(kind); await f.select("node"); f.copy(); finish(new Blob([png], {type: "image/png"}));
  const html = await (await write.mock.calls[0]![0][0]!.getType("text/html")).text();
  const event = new Event("paste", {bubbles: true, cancelable: true});
  Object.defineProperty(event, "clipboardData", {value: {types: ["text/html", "Files"],
    getData: (mime: string) => mime === "text/html" ? html : "", files: [new File([png], "image.png", {type: "image/png"})], items: []}});
  await act(async () => {f.editor.setTextCursorPosition("text", "end"); f.editor.domElement!.dispatchEvent(event);});
  expect(f.onFiles).not.toHaveBeenCalled();
  const images = f.editor.document.filter(block => block.type === "managedImage");
  expect(images).toHaveLength(2);
  expect(images[1]!.props).toMatchObject({assetId: "asset", caption: "caption", previewWidth: 200});
});

it.each((["note", "writing"] as const).flatMap(kind => (["node", "block"] as const).map(selection => ({kind, selection}))))("$kind round trips a $selection image selection when the copy event cannot retain the custom MIME", async ({kind, selection}) => {
  const f = await mount(kind); await f.select(selection);
  f.copy("copy", false); finish(new Blob([png], {type:"image/png"}));
  const html = await (await write.mock.calls[0]![0][0]!.getType("text/html")).text();
  expect(html).toContain('data-content-type="managedImage"');
  expect(html).not.toContain("正在加载图片");
  const event = new Event("paste", {bubbles:true, cancelable:true});
  Object.defineProperty(event, "clipboardData", {value:{types:["text/html"], files:[], items:[],
    getData:(type:string) => type === "text/html" ? html : ""}});
  await act(async () => {f.editor.setTextCursorPosition("text", "end"); f.editor.domElement!.dispatchEvent(event);});
  const images = f.editor.document.filter(block => block.type === "managedImage");
  expect(images).toHaveLength(2);
  expect(images[1]!.props).toMatchObject({assetId:"asset", caption:"caption", previewWidth:200});
  expect(f.onFiles).not.toHaveBeenCalled();
  expect(nativeCopyWrites).toBe(0);
});

it.each(["note", "writing"] as const)("%s restores structured image HTML after the platform removes its marker and rewrites the image URL", async kind => {
  const f = await mount(kind); await f.select("node"); f.copy(); finish(new Blob([png], {type:"image/png"}));
  const html = await (await write.mock.calls[0]![0][0]!.getType("text/html")).text();
  const fragment = new DOMParser().parseFromString(html, "text/html");
  fragment.querySelectorAll("[data-note-image-clipboard]").forEach(node => node.removeAttribute("data-note-image-clipboard"));
  fragment.querySelector("img")!.setAttribute("src", "file:///synthetic-clipboard/image.png");
  const event = new Event("paste", {bubbles:true, cancelable:true});
  Object.defineProperty(event, "clipboardData", {value:{types:["text/html"], files:[], items:[],
    getData:(type:string) => type === "text/html" ? fragment.body.innerHTML : ""}});
  await act(async () => {f.editor.setTextCursorPosition("text", "end"); f.editor.domElement!.dispatchEvent(event);});
  expect(f.onFiles).not.toHaveBeenCalled();
  const images = f.editor.document.filter(block => block.type === "managedImage");
  expect(images).toHaveLength(2);
  expect(images[1]!.props).toMatchObject({assetId:"asset", caption:"caption", previewWidth:200});
});

it.each(["note", "writing"] as const)("%s imports a standard embedded PNG when no custom block data or clipboard File survives", async kind => {
  const f = await mount(kind);
  const event = new Event("paste", {bubbles:true, cancelable:true});
  const html = `<p><img src="data:image/png;base64,${btoa(String.fromCharCode(...png))}"></p>`;
  Object.defineProperty(event, "clipboardData", {value:{types:["text/html"], files:[], items:[], getData:(type:string) => type === "text/html" ? html : ""}});
  await act(async () => {f.editor.setTextCursorPosition("text", "end"); f.editor.domElement!.dispatchEvent(event);});
  expect(f.onFiles).toHaveBeenCalledOnce();
  const [files, insertion] = f.onFiles.mock.calls[0]!;
  expect(files).toHaveLength(1); expect(files[0].type).toBe("image/png");
  expect(new Uint8Array(await files[0].arrayBuffer())).toEqual(png);
  expect(insertion).toEqual({blockId:"text", placement:"replaceEmpty"});
});

it("does not fetch external clipboard image URLs or discard accompanying prose", () => {
  expect(embeddedImageClipboardFile('<img src="file:///private/image.png">')).toBeUndefined();
  expect(embeddedImageClipboardFile('<img src="https://example.invalid/image.png">')).toBeUndefined();
  expect(embeddedImageClipboardFile(`<p>Keep this text<img src="data:image/png;base64,${btoa(String.fromCharCode(...png))}"></p>`)).toBeUndefined();
});

it("reports invalid embedded PNG data rather than silently swallowing the paste", async () => {
  const f = await mount("note");
  const event = new Event("paste", {bubbles:true, cancelable:true});
  Object.defineProperty(event,"clipboardData",{value:{types:["text/html"],files:[],items:[],
    getData:(type:string) => type === "text/html" ? '<img src="data:image/png;base64,%%%">' : ""}});
  await act(async () => {f.editor.setTextCursorPosition("text","end"); f.editor.domElement!.dispatchEvent(event);});
  expect(event.defaultPrevented).toBe(true); expect(f.onFiles).not.toHaveBeenCalled();
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("NOTE_IMAGE_PASTE_DECODE_FAILED"));
});

it("does not insert an embedded clipboard image into a read-only editor", async () => {
  const f = await mount("note", true);
  const event = new Event("paste", {bubbles:true, cancelable:true});
  Object.defineProperty(event,"clipboardData",{value:{types:["text/html"],files:[],items:[],
    getData:(type:string) => type === "text/html" ? `<img src="data:image/png;base64,${btoa(String.fromCharCode(...png))}">` : ""}});
  await act(async () => f.editor.domElement!.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(true); expect(f.onFiles).not.toHaveBeenCalled();
});
