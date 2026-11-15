// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {NodeSelection, TextSelection} from "@tiptap/pm/state";
import {closeHistory} from "@tiptap/pm/history";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {notePageId} from "@research-assistant/core/notes";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {BlockCaption} from "./BlockCaption";
import {blockNoteToNoteDocument, noteDocumentToBlockNote} from "../../features/notes/editor/blockNoteAdapter";
import {noteEditorSchema, type NoteEditorInstance, type PartialNoteEditorBlock} from "../../features/notes/editor/noteEditorSchema";
import {writingNoteSchema, WritingNoteOwner} from "../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteSchema";
import {NotesProvider} from "../../features/notes/NotesClient";
import {noteRendererGateway} from "../../../app/previews/hbs-adapters/noteGateway";
import {replaceBlockText, textBlockLocation} from "./blockTextEditing";
import {BLOCK_CAPTION, captionBlockId} from "./captionBlockProjection";
import {captionNodeText} from "./captionBlock";
import {ManagedAssetIngress} from "./ManagedAssetIngress";

vi.mock("../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteAssets", () => ({
  readWritingNoteAsset: async () => Uint8Array.of(1), exportWritingNoteAsset: vi.fn(),
}));
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:caption-image");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
async function key(element: HTMLElement, value: string, options: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", {key: value, bubbles: true, cancelable: true, ...options});
  await act(async () => element.dispatchEvent(event)); return event;
}
async function fixture(kind: "notes" | "writing", type: "image" | "table", caption = "原说明") {
  const media: PartialNoteEditorBlock = type === "image" ? {id: "media", type: "managedImage", props: {
    assetId: "a".repeat(64), displayName: "image.png", mimeType: "image/png", caption,
  }} : {id: "media", type: "table", props: {caption}, content: {type: "tableContent", rows: [{cells: ["A", "B"]}]}};
  const editor = BlockNoteEditor.create({schema: kind === "notes" ? noteEditorSchema : writingNoteSchema,
    initialContent: [{id: "before", type: "paragraph", content: "Before"}, media, {id: "after", type: "paragraph", content: "After"}],
  }) as unknown as NoteEditorInstance;
  const gateway = new Proxy(noteRendererGateway, {get(target, name) {
    return name === "readNoteAsset" ? async () => Uint8Array.of(1) : Reflect.get(target, name);
  }});
  const imported = vi.fn();
  const render = async (readOnly = false) => {
    await act(async () => root.render(<NotesProvider gateway={gateway}><WritingNoteOwner.Provider value="fixture">
      <NoteEditorSurface editor={editor} readOnly={readOnly} onChange={() => undefined}>
        <ManagedAssetIngress editor={editor} onFiles={imported}/>
      </NoteEditorSurface>
    </WritingNoteOwner.Provider></NotesProvider>));
  };
  await render();
  await vi.waitFor(() => expect(editor.getBlock(captionBlockId("media"))?.type).toBe(caption ? BLOCK_CAPTION : undefined));
  const location = () => textBlockLocation(editor.prosemirrorView.state.doc, captionBlockId("media"))!;
  const at = async (offset = 0, to = offset) => {
    await act(async () => {
      const view = editor.prosemirrorView, start = location().pos + 2;
      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, start + offset, start + to)));
      view.focus();
    });
  };
  const replace = async (text: string) => {await act(async () => {
    const view = editor.prosemirrorView;
    view.dispatch(closeHistory(view.state.tr));
    const tr = view.state.tr; expect(replaceBlockText(tr, captionBlockId("media"), text)).toBe(true); view.dispatch(tr);
  });};
  return {editor, imported, render, at, replace, location,
    text: () => captionNodeText(location().node.firstChild!),
    caption: () => {const props = editor.getBlock("media")?.props; return String(props && "caption" in props ? props.caption : "");},
    field: () => host.querySelector<HTMLElement>('[data-content-type="blockCaption"] .block-caption-input')!,
    save: () => blockNoteToNoteDocument(notePageId("caption"), editor.document),
  };
}

for (const kind of ["notes", "writing"] as const) for (const type of ["image", "table"] as const) {
  it(`edits ${type} caption as a native text block in the one ${kind} document`, async () => {
    const f = await fixture(kind, type), view = f.editor.prosemirrorView, table = host.querySelector("table");
    expect(view.dom.contains(f.field())).toBe(true);
    expect(f.field().getAttribute("contenteditable")).toBeNull();
    expect(host.querySelector("textarea")).toBeNull();
    await f.at(1);
    await act(async () => f.field().dispatchEvent(new MouseEvent("click", {bubbles: true})));
    expect(view.state.selection.$from.parent.type.name).toBe(BLOCK_CAPTION);
    expect(view.state.selection.$from.parentOffset).toBe(1);
    await f.replace("图 1\n两行说明");
    expect(f.caption()).toBe("图 1\n两行说明");
    expect(f.text()).toBe("图 1\n两行说明");
    if (type === "table") expect(host.querySelector("table")).toBe(table);
    expect(f.save().blocks[1]?.children).toEqual([]);
    const reopened = BlockNoteEditor.create({schema: kind === "notes" ? noteEditorSchema : writingNoteSchema,
      initialContent: noteDocumentToBlockNote(f.save())}) as unknown as NoteEditorInstance;
    expect(reopened.getBlock(captionBlockId("media"))?.type).toBe(BLOCK_CAPTION);
    expect(blockNoteToNoteDocument(notePageId("caption"), reopened.document)).toEqual(f.save());
  });

  it(`splits ${type} caption with the shared Enter command in ${kind}, including native undo/redo`, async () => {
    const f = await fixture(kind, type);
    await f.at(1); await key(f.field(), "Enter", {isComposing: true}); expect(f.caption()).toBe("原说明");
    await key(f.field(), "Enter");
    expect(f.caption()).toBe("原"); expect(f.text()).toBe("原");
    const next = f.editor.document[2]!;
    expect(next.type).toBe("paragraph"); expect(next.content).toMatchObject([{text: "说明"}]);
    expect(f.editor.prosemirrorState.selection.$from.parent.type.name).toBe("paragraph");
    expect(f.editor.getTextCursorPosition().block.id).toBe(next.id);
    expect(f.editor.document[1]?.children.map(block => block.type)).toEqual([BLOCK_CAPTION]);
    await act(async () => {expect(f.editor.undo()).toBe(true);});
    expect(f.caption()).toBe("原说明"); expect(f.editor.document.map(block => block.id)).toEqual(["before", "media", "after"]);
    await act(async () => {expect(f.editor.redo()).toBe(true);}); expect(f.caption()).toBe("原");
  });

  it(`uses native Shift+Enter, pure-text paste, and host history for ${type} in ${kind}`, async () => {
    const f = await fixture(kind, type); await f.at(3);
    await key(f.field(), "Enter", {shiftKey: true});
    expect(f.caption()).toBe("原说明\n"); expect(f.editor.document).toHaveLength(3);
    const paste = new Event("paste", {bubbles: true, cancelable: true});
    Object.defineProperty(paste, "clipboardData", {value: {getData: (format: string) => format === "text/plain" ? "纯文本\r\n下一行" : "<strong>格式</strong>"}});
    await act(async () => f.field().dispatchEvent(paste));
    expect(f.caption()).toBe("原说明\n纯文本\n下一行");
    expect(f.field().querySelector("strong,a")).toBeNull();
    await f.replace("新说明");
    await act(async () => {expect(f.editor.undo()).toBe(true);}); expect(f.caption()).toBe("原说明\n纯文本\n下一行");
    await act(async () => {expect(f.editor.redo()).toBe(true);}); expect(f.caption()).toBe("新说明");
    await act(async () => f.editor.updateBlock("media", {props: {caption: "外部更新"}}));
    expect(f.text()).toBe("外部更新");
    await f.render(true); expect(f.editor.isEditable).toBe(false); expect(f.text()).toBe("外部更新");
    expect(f.editor.prosemirrorView.dom.getAttribute("contenteditable")).toBe("false");
  });

  it(`joins a split ${type} caption and following body with the same boundary command in ${kind}`, async () => {
    const f = await fixture(kind, type); await f.at(1); await key(f.field(), "Enter");
    const next = f.editor.document[2]!;
    await act(async () => f.editor.setTextCursorPosition(next.id, "start"));
    await key(f.editor.domElement!, "Backspace");
    expect(f.caption()).toBe("原说明"); expect(f.editor.document.map(block => block.id)).toEqual(["before", "media", "after"]);
    expect(f.editor.prosemirrorState.selection.$from.parent.type.name).toBe(BLOCK_CAPTION);
    expect(f.editor.prosemirrorState.selection.$from.parentOffset).toBe(1);
  });

  it(`positions an empty ${type} caption before typing or changing selection in ${kind}`, async () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const hidden = !!this.closest(".block-caption-hidden");
      const left = hidden ? 0 : 100;
      return {left, right: left + (hidden ? 0 : 600), width: hidden ? 0 : 600,
        top: 20, bottom: 60, height: 40, x: left, y: 20, toJSON() {return {};}};
    });
    const f = await fixture(kind, type, "");
    const content = () => f.field().parentElement!;
    expect(f.editor.getBlock(captionBlockId("media"))).toBeUndefined();
    await act(async () => host.querySelector<HTMLButtonElement>(`button[aria-label="编辑${type === "image" ? "图片" : "表格"}说明"]`)!.click());
    expect(host.querySelector(".block-caption-hidden")).toBeNull();
    expect(content().style.marginLeft).toBe("0px");
    expect(content().style.width).toBe("600px");
    await f.replace("说明");
    expect(content().style.marginLeft).toBe("0px");
  });

  it(`excludes a hidden ${type} caption from native Backspace in ${kind}`, async () => {
    const f = await fixture(kind, type, "");
    const view = f.editor.prosemirrorView;
    await act(async () => f.editor.setTextCursorPosition("after", "start"));
    await key(view.dom, "Backspace");
    expect(f.editor.getBlock(captionBlockId("media"))).toBeUndefined();
    expect(view.state.selection.$from.parent.type.name).not.toBe(BLOCK_CAPTION);
    expect(f.save().blocks.some(block => block.content.kind === "inline"
      && block.content.items.some(item => item.type === "text" && item.text === "After"))).toBe(true);
  });

  it(`keeps an empty ${type} caption absent after beforeinput deletes the next empty paragraph in ${kind}`, async () => {
    const f = await fixture(kind, type, "");
    await act(async () => {
      f.editor.updateBlock("after", {content: []});
      f.editor.setTextCursorPosition("after", "start");
      f.editor.domElement!.dispatchEvent(new InputEvent("beforeinput", {
        inputType: "deleteContentBackward", bubbles: true, cancelable: true,
      }));
    });
    expect(f.editor.getBlock(captionBlockId("media"))).toBeUndefined();
    expect(f.editor.prosemirrorState.selection.$from.parent.type.name).not.toBe(BLOCK_CAPTION);
    expect(f.editor.getBlock("media")).toBeDefined();
  });

  it(`removes a cleared ${type} caption on exit and can explicitly reopen it in ${kind}`, async () => {
    const f = await fixture(kind, type);
    await f.at(); await f.replace("");
    expect(f.editor.getBlock(captionBlockId("media"))?.type).toBe(BLOCK_CAPTION);
    await act(async () => f.editor.setTextCursorPosition("after", "start"));
    expect(f.editor.getBlock(captionBlockId("media"))).toBeUndefined();
    const saved = f.save();
    await act(async () => host.querySelector<HTMLButtonElement>(`button[aria-label="编辑${type === "image" ? "图片" : "表格"}说明"]`)!.click());
    expect(f.editor.prosemirrorState.selection.$from.parent.type.name).toBe(BLOCK_CAPTION);
    expect(f.text()).toBe("");
    expect(f.save()).toEqual(saved);
    await f.replace("重新填写"); expect(f.caption()).toBe("重新填写");
  });

  it(`restores a cleared and dismissed ${type} caption through native undo in ${kind}`, async () => {
    const f = await fixture(kind, type);
    await f.at(); await f.replace("");
    await act(async () => f.editor.setTextCursorPosition("after", "start"));
    expect(f.editor.getBlock(captionBlockId("media"))).toBeUndefined();
    await act(async () => {expect(f.editor.undo()).toBe(true);});
    expect(f.caption()).toBe("原说明"); expect(f.text()).toBe("原说明");
    await act(async () => {expect(f.editor.redo()).toBe(true);});
    expect(f.caption()).toBe("");
  });

  it(`keeps IME confirmation in the ${type} caption and admits the next deliberate Enter in ${kind}`, async () => {
    const f = await fixture(kind, type, "biao ge"), field = f.field();
    await f.at(7);
    await act(async () => {
      field.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true}));
      field.dispatchEvent(new CompositionEvent("compositionend", {bubbles: true, data: "表格"}));
    });
    const paragraph = new InputEvent("beforeinput", {bubbles: true, cancelable: true,
      inputType: "insertParagraph", isComposing: false});
    await act(async () => field.dispatchEvent(paragraph));
    expect(paragraph.defaultPrevented).toBe(true);
    expect(f.editor.prosemirrorState.selection.$from.parent.type.name).toBe(BLOCK_CAPTION);
    await act(async () => {
      field.textContent = "表格";
      field.dispatchEvent(new InputEvent("input", {bubbles: true, inputType: "insertText", data: "表格"}));
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    });
    expect(f.caption()).toBe("表格");
    expect(f.editor.document.map(block => block.id)).toEqual(["before", "media", "after"]);
    expect(f.field()).toBe(field);
    await f.at(2); await key(field, "Enter");
    expect(f.editor.prosemirrorState.selection.$from.parent.type.name).toBe("paragraph");
    expect(f.editor.document).toHaveLength(4);
    expect(f.caption()).toBe("表格");
  });
}

it("keeps caption input and resource deletion in the same native document history", async () => {
  const f = await fixture("notes", "image"); await f.at(1);
  await act(async () => {
    f.field().firstChild!.textContent = "浏览器输入后的说明";
    f.field().dispatchEvent(new InputEvent("input", {bubbles: true, inputType: "insertText", data: "说明"}));
    await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
  });
  expect(f.caption()).toBe("浏览器输入后的说明");
  await act(async () => {f.editor.prosemirrorView.dispatch(closeHistory(f.editor.prosemirrorState.tr)); f.editor.removeBlocks(["media"]);});
  expect(f.editor.getBlock(captionBlockId("media"))).toBeUndefined();
  await act(async () => {f.editor.undo();}); expect(f.caption()).toBe("浏览器输入后的说明");
});

it("rejects caption text past the field limit and prevents file paste importing a body image", async () => {
  const f = await fixture("notes", "image"); await f.at();
  const view = f.editor.prosemirrorView;
  await act(async () => view.dispatch(view.state.tr.insertText("x".repeat(4001))));
  expect(f.caption()).toBe("原说明");
  const paste = new Event("paste", {bubbles: true, cancelable: true});
  const file = new File([Uint8Array.of(1)], "screenshot.png", {type: "image/png"});
  Object.defineProperty(paste, "clipboardData", {value: {types: ["Files"], files: [file], items: [], getData: () => ""}});
  await act(async () => f.field().dispatchEvent(paste));
  expect(f.imported).not.toHaveBeenCalled(); expect(f.caption()).toBe("原说明");
});

it("focuses an existing native empty caption from the media control", async () => {
  const f = await fixture("notes", "image", "");
  expect(f.editor.getBlock(captionBlockId("media"))).toBeUndefined();
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="编辑图片说明"]')!.click());
  expect(f.editor.prosemirrorState.selection.$from.parent.type.name).toBe(BLOCK_CAPTION);
  expect(host.querySelector(".block-caption-hidden")).toBeNull();
  expect(f.field().dataset.placeholder).toBe("Write a caption…");
  await act(async () => f.editor.setTextCursorPosition("after", "start"));
  expect(f.editor.getBlock(captionBlockId("media"))).toBeUndefined();
});

it("renders standalone export captions as static text only", async () => {
  await act(async () => root.render(<BlockCaption value="图 1"/>));
  expect(host.textContent).toBe("图 1"); expect(host.querySelector("[contenteditable],button")).toBeNull();
  await act(async () => root.render(<BlockCaption value=""/>)); expect(host.textContent).toBe("");
});

it("copies a resource with its native caption and reattaches it to a new resource identity on paste", async () => {
  const f = await fixture("notes", "image"), view = f.editor.prosemirrorView;
  const owner = textBlockLocation(view.state.doc, "media")!;
  const selection = NodeSelection.create(view.state.doc, owner.pos);
  const html = view.serializeForClipboard(selection.content()).dom.innerHTML;
  await act(async () => {f.editor.setTextCursorPosition("after", "end");});
  const paste = new Event("paste", {bubbles: true, cancelable: true});
  Object.defineProperty(paste, "clipboardData", {value: {types: ["blocknote/html", "text/html", "text/plain"], files: [],
    getData: (type: string) => type === "text/plain" ? "原说明" : html}});
  await act(async () => f.editor.domElement!.dispatchEvent(paste));
  const images = f.editor.document.filter(block => block.type === "managedImage");
  expect(images).toHaveLength(2); expect(images[1]?.id).not.toBe(images[0]?.id);
  expect(images[1]?.children).toMatchObject([{id: captionBlockId(images[1]!.id), type: BLOCK_CAPTION}]);
  expect(f.save().blocks.filter(block => block.type === "image").map(block => block.content)).toEqual([f.save().blocks[1]!.content, f.save().blocks[1]!.content]);
});

it("exports a mounted table caption once and retains its portable caption field", async () => {
  const f = await fixture("notes", "table");
  const html = f.editor.blocksToFullHTML();
  const parsed = new DOMParser().parseFromString(html, "text/html");
  expect(parsed.body.textContent?.match(/原说明/gu)).toHaveLength(1);
  expect((await f.editor.tryParseHTMLToBlocks(html)).find(block => block.type === "table")?.props).toMatchObject({caption: "原说明"});
});
