// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {HbsSession, createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {noteBlockId, notePageId, type NoteDocumentV1} from "@research-assistant/core/notes";
import {NotesGatewayError, type NoteAsset, type NoteAssetImportRequest} from "@research-assistant/data/notes";
import {NotesProvider} from "../NotesClient";
import {noteRendererGateway} from "../../../../app/previews/hbs-adapters/noteGateway";
import {notify} from "../../../services/notifications";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {HbsNoteEditor} from "../../../components/hbs/HbsNoteEditor";
import {NoteEditor} from "./NoteEditor";
import {blockNoteToNoteDocument} from "./blockNoteAdapter";
import type {NoteEditorInstance} from "./noteEditorSchema";
import {BlockSetSelection} from "../../../components/block-editor/BlockSetSelection";

vi.mock("../../../services/notifications", () => ({notify: {error: vi.fn(), info: vi.fn(), success: vi.fn()}}));
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jI1sAAAAASUVORK5CYII="), value => value.charCodeAt(0));
const imageId = "a".repeat(64);
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const dispose of cleanups.splice(0)) await dispose();
  vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks();
});
function imageFile(name = "image.png") {
  const file = new File([png], name, {type: "image/png"});
  Object.defineProperty(file, "arrayBuffer", {value: async () => png.slice().buffer});
  return file;
}
function pasteFiles(editor: NoteEditorInstance, files: readonly File[]) {
  const event = new Event("paste", {bubbles: true, cancelable: true});
  Object.defineProperty(event, "clipboardData", {value: {
    types: ["Files"], files,
    items: files.map(file => ({kind: "file", type: file.type, getAsFile: () => file})),
    getData: () => "",
  }});
  editor.domElement!.dispatchEvent(event);
}
async function setup(importer?: (request: NoteAssetImportRequest) => Promise<NoteAsset>, saved?: NoteDocumentV1, hbs = false) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let importedCount = 0;
  const imported = vi.fn(importer ?? (async request => {
    const id = importedCount++ === 0 ? imageId : "b".repeat(64);
    return {id, displayName: `${id}.png`, mimeType: request.mimeType, byteSize: request.bytes.length};
  }));
  const read = vi.fn(async () => png.slice());
  const objectUrl = vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:managed-note-image");
  const revoke = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  const gateway = new Proxy(noteRendererGateway, {get(target, key) {
    if (key === "importNoteAsset") return imported;
    if (key === "readNoteAsset") return read;
    return Reflect.get(target, key);
  }});
  const initial: NoteDocumentV1 = saved ?? {schemaVersion: 1, pageId: notePageId("image-note"), blocks: [{
    id: noteBlockId("text"), type: "paragraph", props: {}, content: {kind: "inline", items: []}, children: [],
  }]};
  let authority = {version: 1, saved: initial, baseline: initial};
  const session = hbs ? await HbsSession.open({adapter: createNoteDocumentAdapter(directTextDiff), requestId: n => `image-history-${n}`,
    preservation: {load: () => undefined, save: () => {}},
    port: {read: async () => authority, commit: async request => {
      authority = {version: authority.version + 1, saved: request.saved, baseline: request.baseline};
      return authority;
    }},
  }) : undefined;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let editor: NoteEditorInstance | undefined;
  const onChange = vi.fn();
  const render = async (readOnly = false) => {
    await act(async () => root.render(<NotesProvider gateway={gateway}>{session
      ? <HbsNoteEditor session={session} isReadOnly={readOnly} onEditorReady={value => {editor = value;}}/>
      : <NoteEditor document={initial} isReadOnly={readOnly} onChange={onChange} onEditorReady={value => {editor = value;}}/>
    }</NotesProvider>));
    await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
  };
  cleanups.push(async () => {await act(async () => root.unmount()); session?.dispose(); host.remove();});
  await render();
  return {host, imported, read, objectUrl, revoke, render, onChange,
    get editor() {if (!editor) throw new Error("Editor not mounted"); return editor;},
    async paste(files: readonly File[]) {await act(async () => {pasteFiles(this.editor, files);});},
    document() {return blockNoteToNoteDocument(initial.pageId, this.editor.document);},
  };
}

it("pastes screenshot files as managed images with durable identity and a resolved blob preview", async () => {
  const f = await setup();
  await f.paste([imageFile()]);
  await vi.waitFor(() => expect(f.imported).toHaveBeenCalledOnce());
  await act(async () => {await Promise.resolve();});
  const image = f.document().blocks.find(block => block.type === "image");
  expect(image?.content).toEqual({kind: "asset", assetId: imageId, displayName: `${imageId}.png`, mimeType: "image/png", caption: ""});
  expect(f.editor.document.find(block => block.type === "managedImage")).toBeDefined();
  expect(f.host.querySelector(".managed-note-asset img")?.getAttribute("src")).toBe("blob:managed-note-image");
  expect(f.host.querySelector(`img[src='${imageId}']`)).toBeNull();
  expect(f.read).toHaveBeenCalledWith(imageId);
  expect(f.imported.mock.calls[0]![0].bytes).toEqual([...png]);
  expect(f.imported.mock.calls[0]![0].displayName).toBe("image");
});

it("imports a clipboard image carried only by standard HTML through the same durable resource path", async () => {
  const f = await setup();
  const html = `<p><img src="data:image/png;base64,${btoa(String.fromCharCode(...png))}"></p>`;
  const event = new Event("paste", {bubbles:true,cancelable:true});
  Object.defineProperty(event,"clipboardData",{value:{types:["text/html"],files:[],items:[],
    getData:(type:string) => type === "text/html" ? html : ""}});
  await act(async () => {f.editor.setTextCursorPosition("text","end");f.editor.domElement!.dispatchEvent(event);});
  await vi.waitFor(() => expect(f.imported).toHaveBeenCalledOnce());
  await act(async () => {await Promise.resolve();});
  expect(f.imported.mock.calls[0]![0].bytes).toEqual([...png]);
  expect(f.document().blocks.find(block => block.type === "image")?.content)
    .toMatchObject({kind:"asset",assetId:imageId,mimeType:"image/png"});
  expect(f.host.querySelector(".managed-note-asset img")?.getAttribute("src")).toBe("blob:managed-note-image");
  expect(event.defaultPrevented).toBe(true);
});

it("copies between production note editors without custom MIME or a second resource import", async () => {
  const source = await setup(); await source.paste([imageFile()]);
  await vi.waitFor(() => expect(source.document().blocks.some(block => block.type === "image")).toBe(true));
  const image = source.editor.document.find(block => block.type === "managedImage")!;
  await act(async () => source.editor.updateBlock(image.id, {props: {caption:"Existing caption", previewScale:45, textAlignment:"right"}}));
  const pixels = source.host.querySelector<HTMLImageElement>(".managed-note-asset img")!;
  Object.defineProperties(pixels, {complete:{value:true}, naturalWidth:{value:800}, naturalHeight:{value:300}});
  vi.spyOn(HTMLCanvasElement.prototype,"getContext").mockReturnValue({drawImage:vi.fn()} as unknown as CanvasRenderingContext2D);
  vi.spyOn(HTMLCanvasElement.prototype,"toBlob").mockImplementation(callback => callback(new Blob([png], {type:"image/png"})));
  let representations: Record<string, Blob | Promise<Blob>> = {};
  vi.stubGlobal("ClipboardItem", class {constructor(data: typeof representations) {representations = data;}});
  const write = vi.spyOn(navigator.clipboard,"write").mockResolvedValue(undefined);
  const values = new Map<string,string>(), copy = new Event("copy", {bubbles:true,cancelable:true});
  Object.defineProperty(copy,"clipboardData",{value:{getData:(type:string) => values.get(type) ?? "",
    setData:(type:string,value:string) => {if (type !== "blocknote/html") values.set(type,value);}, clearData:() => values.clear()}});
  await act(async () => {
    const view = source.editor.prosemirrorView;
    view.dispatch(view.state.tr.setSelection(BlockSetSelection.create(view.state.doc,[image.id])));
    source.editor.domElement!.dispatchEvent(copy);
  });
  expect(write).toHaveBeenCalledOnce(); expect(values.size).toBe(0);
  expect((await representations["image/png"]!).type).toBe("image/png");
  const html = await (await representations["text/html"]!).text(), destination = await setup();
  const paste = new Event("paste", {bubbles:true,cancelable:true});
  Object.defineProperty(paste,"clipboardData",{value:{types:["text/html"],files:[],items:[],
    getData:(type:string) => type === "text/html" ? html : ""}});
  await act(async () => {
    destination.editor.setTextCursorPosition("text","start");
    destination.host.querySelector('[data-id="text"] .bn-inline-content')!.dispatchEvent(paste);
  });
  expect(destination.imported).not.toHaveBeenCalled();
  const restored = destination.document().blocks.find(block => block.type === "image");
  expect(restored?.content).toMatchObject({kind:"asset",assetId:imageId,caption:"Existing caption"});
  expect(restored?.props).toMatchObject({previewScale:45,textAlignment:"right"});
  expect(destination.host.querySelector(".managed-note-asset img")?.getAttribute("src")).toBe("blob:managed-note-image");
});

it("copies an image inside the same HBS note with a new block identity and one native undo step", async () => {
  const f = await setup(undefined, undefined, true);
  await f.paste([imageFile()]);
  await vi.waitFor(() => expect(f.editor.document.some(block => block.type === "managedImage")).toBe(true));
  const original = f.editor.document.find(block => block.type === "managedImage")!;
  let html = "";
  await act(async () => {
    f.editor.updateBlock(original.id, {props:{caption:"Copied caption",previewScale:45,textAlignment:"right"}});
    f.editor.insertBlocks([{id:"paste-target",type:"paragraph",content:""}], original.id, "after");
  });
  await act(async () => {
    const view = f.editor.prosemirrorView;
    view.dispatch(view.state.tr.setSelection(BlockSetSelection.create(view.state.doc,[original.id])));
    html = view.serializeForClipboard(view.state.selection.content()).dom.innerHTML;
    f.editor.setTextCursorPosition("paste-target", "start");
  });
  const paste = new Event("paste", {bubbles:true,cancelable:true});
  Object.defineProperty(paste, "clipboardData", {value:{types:["text/html","text/plain","Files"],files:[imageFile()],items:[],
    getData:(type:string)=>type==="text/html"?html:""}});
  await act(async () => f.editor.domElement!.dispatchEvent(paste));
  const images = f.document().blocks.filter(block => block.type === "image");
  expect(images).toHaveLength(2);
  expect(new Set(images.map(block=>block.id)).size).toBe(2);
  expect(images.map(block=>block.content.kind==="asset"?block.content.assetId:undefined)).toEqual([imageId,imageId]);
  for (const image of images) {
    expect(image.props).toMatchObject({previewScale:45,textAlignment:"right"});
    expect(image.content).toMatchObject({caption:"Copied caption"});
  }
  expect(f.imported).toHaveBeenCalledOnce();
  await act(async () => {f.editor.undo();});
  expect(f.document().blocks.filter(block=>block.type==="image").map(block=>block.id)).toEqual([original.id]);
});

it("does not leave a blank or broken image block after a screenshot import fails", async () => {
  const f = await setup(async () => {throw new NotesGatewayError("storage_error", "image import disk failure");});
  await f.paste([imageFile()]);
  await vi.waitFor(() => expect(notify.error).toHaveBeenCalledWith("image import disk failure"));
  expect(f.document().blocks.some(block => block.type === "image" || block.type === "file")).toBe(false);
  expect(f.host.querySelector("img")).toBeNull();
});

it("does not insert an imported screenshot after the same note becomes read-only", async () => {
  let finish!: (asset: NoteAsset) => void;
  const f = await setup(() => new Promise(resolve => {finish = resolve;}));
  await f.paste([imageFile()]);
  await vi.waitFor(() => expect(f.imported).toHaveBeenCalledOnce());
  await f.render(true);
  await act(async () => {finish({id: imageId, displayName: "image.png", mimeType: "image/png", byteSize: png.length});});
  expect(f.document().blocks.some(block => block.type === "image" || block.type === "file")).toBe(false);
});

it("preserves all images and their clipboard order when multiple screenshots are pasted", async () => {
  const f = await setup();
  await f.paste([imageFile("first.png"), imageFile("second.png")]);
  await vi.waitFor(() => expect(f.imported).toHaveBeenCalledTimes(2));
  await act(async () => {await Promise.resolve();});
  const names = f.document().blocks.flatMap(block => block.content.kind === "asset" ? [block.content.displayName] : []);
  expect(names).toEqual([`${imageId}.png`, `${"b".repeat(64)}.png`]);
  expect(f.editor.document.filter(block => block.type === "managedImage")).toHaveLength(2);
});

it("uses the same managed image shape and blob renderer for the file picker", async () => {
  const f = await setup(), input = f.host.querySelector<HTMLInputElement>('input[type="file"]')!;
  Object.defineProperty(input, "files", {configurable: true, value: [imageFile("picked.png")]});
  await act(async () => {input.dispatchEvent(new Event("change", {bubbles: true}));});
  await vi.waitFor(() => expect(f.imported).toHaveBeenCalledOnce());
  await act(async () => {await Promise.resolve();});
  expect(f.editor.document.some(block => block.type === "managedImage")).toBe(true);
  expect(f.document().blocks.find(block => block.type === "image")?.content).toEqual({
    kind: "asset", assetId: imageId, displayName: `${imageId}.png`, mimeType: "image/png", caption: "",
  });
  expect(f.host.querySelector(".managed-note-asset img")?.getAttribute("src")).toBe("blob:managed-note-image");
});

it("drops an image next to the targeted block rather than the editor's old cursor", async () => {
  const f = await setup();
  await act(async () => {
    f.editor.updateBlock("text", {content: "First"});
    f.editor.insertBlocks([{id: "drop-target", type: "paragraph", content: "Target"},
      {id: "after-target", type: "paragraph", content: "After"}], "text", "after");
    f.editor.setTextCursorPosition("text", "start");
  });
  let position = 0;
  f.editor.prosemirrorView.state.doc.descendants((node, pos) => {if (node.attrs.id === "drop-target") position = pos + 2;});
  vi.spyOn(f.editor.prosemirrorView, "posAtCoords").mockReturnValue({pos: position, inside: -1});
  // BlockNote's document-level drop owner clamps events to the editor bounds.
  // happy-dom has no layout; give it the same enclosing geometry as the blocks.
  vi.spyOn(f.editor.domElement!, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 500, 600));
  vi.spyOn(f.editor.domElement!.firstElementChild!, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 500, 600));
  const target = f.host.querySelector<HTMLElement>('[data-id="drop-target"]')!;
  vi.spyOn(target, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 100, 400, 40));
  const event = new Event("drop", {bubbles: true, cancelable: true}), file = imageFile("dropped.png");
  Object.defineProperties(event, {
    clientX: {value: 100}, clientY: {value: 135},
    dataTransfer: {value: {types: ["Files"], files: [file], items: [{kind: "file", type: file.type, getAsFile: () => file}], getData: () => "", effectAllowed: "copy"}},
  });
  await act(async () => {target.dispatchEvent(event);});
  await vi.waitFor(() => expect(f.imported).toHaveBeenCalledOnce());
  await act(async () => {await Promise.resolve();});
  const blocks = f.editor.document, image = blocks.find(block => block.type === "managedImage");
  expect(image).toBeDefined();
  expect(blocks.findIndex(block => block.id === image!.id)).toBe(blocks.findIndex(block => block.id === "drop-target") + 1);
  expect(blocks.findIndex(block => block.id === "after-target")).toBe(blocks.findIndex(block => block.id === image!.id) + 1);
});

it("reopens the portable saved image reference without reuploading or persisting the blob URL", async () => {
  const first = await setup(); await first.paste([imageFile()]);
  await vi.waitFor(() => expect(first.imported).toHaveBeenCalledOnce());
  await act(async () => {await Promise.resolve();});
  const saved = first.document();
  expect(JSON.stringify(saved)).not.toContain("blob:");
  const reopened = await setup(undefined, saved);
  expect(reopened.imported).not.toHaveBeenCalled();
  expect(reopened.read).toHaveBeenCalledWith(imageId);
  expect(reopened.host.querySelector(".managed-note-asset img")?.getAttribute("src")).toBe("blob:managed-note-image");
});

it("publishes no unresolved asset while uploading and preserves text typed before completion", async () => {
  let finish!: (asset: NoteAsset) => void;
  const f = await setup(() => new Promise(resolve => {finish = resolve;}));
  await f.paste([imageFile()]);
  await vi.waitFor(() => expect(f.imported).toHaveBeenCalledOnce());
  await act(async () => {f.editor.updateBlock("text", {content: "Typed while the screenshot was uploading"});});
  const publishedBefore = f.onChange.mock.calls.map(([value]) => value as NoteDocumentV1);
  expect(publishedBefore.flatMap(value => value.blocks.filter(block => block.content.kind === "asset"))).toEqual([]);
  await act(async () => {finish({id: imageId, displayName: "image.png", mimeType: "image/png", byteSize: png.length});});
  const saved = f.document();
  expect(saved.blocks[0]!.content).toEqual({kind: "inline", items: [{type: "text", text: "Typed while the screenshot was uploading", styles: []}]});
  expect(saved.blocks.find(block => block.content.kind === "asset")?.content).toEqual({
    kind: "asset", assetId: imageId, displayName: "image.png", mimeType: "image/png", caption: "",
  });
  for (const [document] of f.onChange.mock.calls) {
    for (const block of (document as NoteDocumentV1).blocks) {
      if (block.content.kind === "asset") expect(block.content.assetId).toBe(imageId);
    }
  }
});

it("undoes and redoes the imported image without losing its identity or importing twice", async () => {
  const f = await setup(undefined, undefined, true); await f.paste([imageFile("history.png")]);
  await vi.waitFor(() => expect(f.host.querySelector(".managed-note-asset img")?.getAttribute("src")).toBe("blob:managed-note-image"));
  const before = f.document().blocks.find(block => block.content.kind === "asset");
  expect(before?.content).toEqual({kind: "asset", assetId: imageId, displayName: `${imageId}.png`, mimeType: "image/png", caption: ""});
  await act(async () => {f.editor.undo();});
  expect(f.document().blocks.some(block => block.content.kind === "asset")).toBe(false);
  expect(f.host.querySelector(".managed-note-asset img")).toBeNull();
  await act(async () => {f.editor.redo();});
  const restored = f.document().blocks.find(block => block.content.kind === "asset");
  expect(restored).toEqual(before);
  await vi.waitFor(() => expect(f.host.querySelector(".managed-note-asset img")?.getAttribute("src")).toBe("blob:managed-note-image"));
  expect(f.imported).toHaveBeenCalledOnce();
  expect(f.read).toHaveBeenCalledTimes(2);
  expect(f.read).toHaveBeenLastCalledWith(imageId);
});
