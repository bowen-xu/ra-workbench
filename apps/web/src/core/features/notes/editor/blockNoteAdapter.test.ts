// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import {BlockNoteEditor} from "@blocknote/core";

import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";

import { blockNoteToNoteDocument, noteDocumentToBlockNote, retainNoteProperties } from "./blockNoteAdapter";
import {noteEditorSchema, type NoteEditorBlock, type NoteEditorInstance} from "./noteEditorSchema";
import {documentTitleBlock, documentTitleId} from "../../../components/block-editor/documentTitle";

it("round-trips automatic page entrances separately from ordinary links",()=>{
  const reference=(id:string,entry:boolean)=>({id:noteBlockId(id),type:"researchObjectReference" as const,props:entry?{pageEntry:true}:{},children:[],content:{kind:"reference" as const,targetKind:"note_page" as const,targetId:"target",labelSnapshot:"Target",quoteSnapshot:""}});
  const document:NoteDocumentV1={schemaVersion:1,pageId:notePageId("entry-codec"),blocks:[reference("automatic",true),reference("ordinary",false)]};
  const editor=BlockNoteEditor.create({schema:noteEditorSchema,initialContent:noteDocumentToBlockNote(document)});
  const saved=blockNoteToNoteDocument(document.pageId,editor.document);
  expect(saved.blocks[0]?.props).toEqual({pageEntry:true});expect(saved.blocks[1]?.props).toEqual({});
});

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});

describe("BlockNote adapter", () => {
  it("decodes the actual native plain title returned by editor.document and its updates", () => {
    const pageId = notePageId("native-title");
    const editor = BlockNoteEditor.create({schema: noteEditorSchema, initialContent: [
      documentTitleBlock(pageId, "真实原生标题"), {id: "body", type: "paragraph", content: "正文"},
    ]}) as unknown as NoteEditorInstance;
    const first = blockNoteToNoteDocument(pageId, editor.document);
    expect(first.title).toBe("真实原生标题"); expect(first.blocks.map(block => block.id)).toEqual(["body"]);
    editor.updateBlock(documentTitleId(pageId), {content: "更新后的标题"});
    expect(blockNoteToNoteDocument(pageId, editor.document).title).toBe("更新后的标题");
  });

  it.each([{content: "public title", expected: "public title"}, {content: [], expected: ""},
    {content: [{type: "text", text: "原生", styles: {}}, {type: "text", text: "标题", styles: {}}], expected: "原生标题"},
  ])("accepts both supported plain title representations %#", ({content, expected}) => {
    const title = {...documentTitleBlock("page", ""), content};
    expect(blockNoteToNoteDocument(notePageId("page"), [title] as unknown as NoteEditorBlock[]).title)
      .toBe(expected);
  });

  it.each([undefined, null, 1, {}, ["text"], [{type: "link", text: "不能丢失链接", styles: {}}],
    [{type: "text", text: "格式", styles: {bold: true}}], [{type: "text", text: "隐藏语义", styles: {}, targetId: "resource"}],
  ].map(content => ({content})))("rejects invalid or semantic title representations without silently flattening them %#", ({content}) => {
    const title = {...documentTitleBlock("page", ""), content};
    expect(() => blockNoteToNoteDocument(notePageId("page"), [title] as unknown as NoteEditorBlock[]))
      .toThrow("note_title_projection_invalid");
  });

  it.each(["readingNoteReference", "researchObjectReference"] as const)("edits historical %s through one link without changing stored identity", type => {
    const document: NoteDocumentV1 = { schemaVersion: 1, pageId: notePageId("historical"), blocks: [{
      id: noteBlockId("reference"), type, props: {}, children: [],
      content: { kind: "reference", targetKind: "reading_note", targetId: "reading", labelSnapshot: "Original", quoteSnapshot: "Retained quote" },
    }] };
    const [link] = noteDocumentToBlockNote(document);
    expect(link?.type).toBe("referenceLink");
    const restored = blockNoteToNoteDocument(document.pageId, [{ ...link, props: { ...link!.props, labelSnapshot: "Edited" } }] as NoteEditorBlock[]);
    expect(restored.blocks[0]).toEqual({ ...document.blocks[0], content: {
      kind: "reference", targetKind: "reading_note", targetId: "reading", labelSnapshot: "Edited", quoteSnapshot: "Retained quote",
    } });
  });
  it("projects snapshot props recursively without losing hidden domain properties", () => {
    const document:NoteDocumentV1={schemaVersion:1,pageId:notePageId("snapshot"),blocks:[{
      id:noteBlockId("parent"),type:"paragraph",props:{textColor:"red",hidden:"retained"},content:{kind:"inline",items:[]},children:[{
        id:noteBlockId("reference"),type:"readingNoteReference",props:{},children:[],
        content:{kind:"reference",targetKind:"reading_note",targetId:"reading",labelSnapshot:"label",quoteSnapshot:"quote"},
      }],
    }]};
    const projected=noteDocumentToBlockNote(document,{
      paragraph:{propSchema:{textColor:{default:"default"}}},
      referenceLink:{propSchema:{referenceType:{default:"researchObjectReference"},targetKind:{default:"paper"},targetId:{default:""},labelSnapshot:{default:""},quoteSnapshot:{default:""}}},
    });
    expect(projected[0]!.props).toEqual({textColor:"red"});
    expect(projected[0]!.children?.[0]?.props).toEqual({referenceType:"readingNoteReference",targetKind:"reading_note",targetId:"reading",labelSnapshot:"label",quoteSnapshot:"quote"});
    expect(retainNoteProperties(blockNoteToNoteDocument(document.pageId,projected as NoteEditorBlock[]),document)).toEqual(document);
    expect(document.blocks[0]!.props.hidden).toBe("retained");
  });
  it("preserves sparse schema defaults while keeping property edits and hidden domain attributes", () => {
    const trusted: NoteDocumentV1 = {schemaVersion:1,pageId:notePageId("page"),blocks:[{
      id:noteBlockId("block"),type:"paragraph",props:{textColor:"red",nativeProperty:"retained"},
      content:{kind:"inline",items:[]},children:[],
    }]};
    const schema={paragraph:{propSchema:{backgroundColor:{default:"default"},textColor:{default:"default"},textAlignment:{default:"left"}}}};
    const native={...trusted,blocks:[{...trusted.blocks[0]!,props:{textColor:"default",backgroundColor:"default",textAlignment:"center"}}]};
    expect(retainNoteProperties(native,trusted,schema).blocks[0]!.props).toEqual({
      textColor:"default",textAlignment:"center",nativeProperty:"retained",
    });
  });
  it("round-trips P0 semantic block content without leaking editor JSON", () => {
    const document: NoteDocumentV1 = {
      schemaVersion: 1,
      pageId: notePageId("page-1"),
      blocks: [
        {
          id: noteBlockId("paragraph"), type: "paragraph", props: {},
          content: { kind: "inline", items: [
            { type: "link", href: "https://example.test", content: [{ type: "text", text: "链接", styles: ["underline"] }] },
            { type: "mention", targetKind: "paper", targetId: "paper-1", labelSnapshot: "论文" },
            { type: "mention", targetKind: "note_page", targetId: "page-2", labelSnapshot: "页面" },
            { type: "mention", targetKind: "reading_note", targetId: "reading-1", labelSnapshot: "阅读笔记" },
          ] }, children: [{
            id: noteBlockId("nested-quote"), type: "quote", props: {},
            content: { kind: "inline", items: [{ type: "text", text: "嵌套", styles: ["italic"] }] }, children: [],
          }],
        },
        {
          id: noteBlockId("heading"), type: "heading", props: { level: 2 },
          content: { kind: "inline", items: [
            { type: "text", text: "结论", styles: ["bold"] },
            { type: "inlineFormula", source: "E=mc^2" },
          ] }, children: [],
        },
        ...(["bulletListItem", "numberedListItem", "checkListItem", "codeBlock"] as const).map((type, index) => ({
          id: noteBlockId(`text-${index}`), type, props: type === "checkListItem" ? { checked: true } : {},
          content: { kind: "inline" as const, items: [{ type: "text" as const, text: type, styles: [] }] }, children: [],
        })),
        {
          id: noteBlockId("divider"), type: "divider", props: {},
          content: { kind: "none" }, children: [],
        },
        {
          id: noteBlockId("table"), type: "table", props: {},
          content: { kind: "table", rows: [[
            { kind: "inline", items: [{ type: "text", text: "A", styles: [] }] },
            { kind: "inline", items: [{ type: "text", text: "B", styles: [] }] },
          ]] }, children: [],
        },
        {
          id: noteBlockId("formula"), type: "formulaBlock", props: {},
          content: { kind: "formula", source: "\\alpha + \\beta" }, children: [],
        },
        {
          id: noteBlockId("asset"), type: "image", props: {},
          content: { kind: "asset", assetId: "sha256", displayName: "figure.png", mimeType: "image/png", caption: "Figure 1" }, children: [],
        },
        {
          id: noteBlockId("file"), type: "file", props: {},
          content: { kind: "asset", assetId: "file-sha", displayName: "data.csv", mimeType: "text/csv", caption: "Data" }, children: [],
        },
        {
          id: noteBlockId("child-page"), type: "childPage", props: {},
          content: { kind: "reference", targetKind: "note_page", targetId: "page-2", labelSnapshot: "子页面", quoteSnapshot: "" }, children: [],
        },
        {
          id: noteBlockId("reference"), type: "readingNoteReference", props: {},
          content: { kind: "reference", targetKind: "reading_note", targetId: "reading-1", labelSnapshot: "Paper", quoteSnapshot: "Quote" }, children: [],
        },
        {
          id: noteBlockId("research-reference"), type: "researchObjectReference", props: {},
          content: { kind: "reference", targetKind: "claim", targetId: "claim-1", labelSnapshot: "Claim", quoteSnapshot: "Result" }, children: [],
        },
        {
          id: noteBlockId("unsupported"), type: "unsupported", props: { originalType: "futureDiagram" },
          content: { kind: "unsupported", originalType: "futureDiagram", rawJson: '{"type":"futureDiagram"}' }, children: [],
        },
      ],
    };

    const editorBlocks = noteDocumentToBlockNote(document) as NoteEditorBlock[];
    const restored = blockNoteToNoteDocument(document.pageId, editorBlocks);
    expect(restored).toEqual(document);
  });

  it("converts pasted or foreign child-page blocks to modern lifecycle entrances", () => {
    const source: NoteDocumentV1 = {
      schemaVersion: 1,
      pageId: notePageId("parent"),
      blocks: [{
        id: noteBlockId("child"), type: "childPage", props: {},
        content: { kind: "reference", targetKind: "note_page", targetId: "child-1", labelSnapshot: "子页面", quoteSnapshot: "" },
        children: [],
      }],
    };
    const [child] = noteDocumentToBlockNote(source) as NoteEditorBlock[];
    const pasted = { ...child, id: "pasted-child" } as NoteEditorBlock;
    const foreign = {
      ...child,
      id: "foreign-child",
      props: { ...(child?.props ?? {}), targetId: "another-parent-child", labelSnapshot: "其他页面" },
    } as NoteEditorBlock;
    const restored = blockNoteToNoteDocument(source.pageId, [pasted, child!, foreign], {
      validChildPages: [{ blockId: String(child!.id), targetId: "child-1" }],
    });
    expect(restored.blocks.map((block) => block.type)).toEqual([
      "researchObjectReference",
      "childPage",
      "researchObjectReference",
    ]);
    expect(restored.blocks[0]?.props.pageEntry).toBe(true);
    expect(restored.blocks[2]?.props.pageEntry).toBe(true);
    expect([restored.blocks[0]?.content, restored.blocks[2]?.content]).toEqual([
      { kind: "reference", targetKind: "note_page", targetId: "child-1", labelSnapshot: "子页面", quoteSnapshot: "" },
      { kind: "reference", targetKind: "note_page", targetId: "another-parent-child", labelSnapshot: "其他页面", quoteSnapshot: "" },
    ]);
  });
});

it("round-trips individually styled reference text and colors without changing its identity", () => {
  const document: NoteDocumentV1 = { schemaVersion: 1, pageId: notePageId("formatted"), blocks: [{
    id: noteBlockId("paragraph"), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{
      type: "mention", targetKind: "note_page", targetId: "target", labelSnapshot: "Original title",
      content: [
        { type: "text", text: "Original ", styles: ["bold", "italic"], textColor: "red" },
        { type: "text", text: "title", styles: ["underline"], backgroundColor: "yellow" },
      ],
    }] },
  }] };
  const editor = noteDocumentToBlockNote(document);
  expect((editor[0]!.content as unknown[])[0]).toMatchObject({ type: "researchMention", props: { targetId: "target" }, content: [
    { text: "Original ", styles: { bold: true, italic: true, textColor: "red" } },
    { text: "title", styles: { underline: true, backgroundColor: "yellow" } },
  ] });
  expect(blockNoteToNoteDocument(document.pageId, editor as NoteEditorBlock[])).toEqual(document);
});
