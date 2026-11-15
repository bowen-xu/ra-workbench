// @vitest-environment happy-dom

import { describe, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import type {NoteExportRequest, NotesGateway} from "@research-assistant/data/notes";

import { MAX_MARKDOWN_IMPORT_BYTES, markdownToNoteDocument, collectNoteAssets, noteDocumentToMarkdown, previewMarkdownImport, safeExportName } from "./noteMarkdown";
import {exportNoteLibrary} from "../../../../../../../plugins/note_manager/src/features/notes/transfer/noteLibraryExport";

function imageDocument(page: string, names: readonly string[]): NoteDocumentV1 {
  return {schemaVersion: 1, pageId: notePageId(page), blocks: names.map((name, i) => ({
    id: noteBlockId(`${page}-${i}`), type: "image", props: {}, children: [],
    content: {kind: "asset", assetId: "a".repeat(64), displayName: name, mimeType: "image/png", caption: `${page} 图 ${i + 1}`},
  }))};
}
function imagePaths(markdown: string): string[] {
  return [...markdown.matchAll(/!\[[^\]]*\]\(([^)]+)\)/gu)].map(match => match[1]!);
}

describe("note Markdown transfer", () => {
  it("imports source links with locators without losing their href", () => {
    const href = "source://library/paper/pdf:#line{3:12-4:14-20}";
    const document = previewMarkdownImport(notePageId("source-import"), `[Paper](${href})`).document;
    expect(noteDocumentToMarkdown(document)).toContain(href);
    expect(JSON.stringify(document)).toContain('"type":"link"');
  });
  it("keeps a standalone quote separate from the preceding paragraph", () => {
    const markdown = "hi\n\n\n\nthis is a note\n\n23\n\n> quote\n\n\n\nToggle list\n\nitem1\n\nitem2\n\n\n\n* item1\n* item2\n* item3\n\n\n\n456\n\n123";
    const document = previewMarkdownImport(notePageId("quote-regression"), markdown).document;
    const quote = document.blocks.find((block) => block.type === "quote");
    expect(quote).toBeDefined();
    expect(document.blocks.every((block) => block.children.length === 0)).toBe(true);
  });

  it("exports research references and managed assets as portable Markdown", () => {
    const document: NoteDocumentV1 = {
      schemaVersion: 1,
      pageId: notePageId("page"),
      blocks: [
        { id: noteBlockId("heading"), type: "heading", props: { level: 1 }, content: { kind: "inline", items: [{ type: "text", text: "Result", styles: ["bold"] }] }, children: [] },
        { id: noteBlockId("asset"), type: "image", props: {}, content: { kind: "asset", assetId: "a".repeat(64), displayName: "plot.png", mimeType: "image/png", caption: "Plot" }, children: [] },
        { id: noteBlockId("ref"), type: "readingNoteReference", props: {}, content: { kind: "reference", targetKind: "reading_note", targetId: "reading", labelSnapshot: "Paper", quoteSnapshot: "quoted" }, children: [] },
      ],
    };
    const markdown = noteDocumentToMarkdown(document);
    expect(markdown).toContain("# **Result**");
    expect(markdown).toContain("../assets/");
    expect(markdown).toContain("source://notes/reading");
    expect(collectNoteAssets(document)).toHaveLength(1);
  });

  it("sanitizes exported filenames", () => {
    expect(safeExportName("../bad:name")).toBe("..-bad-name");
  });

  it("exports images by internal identity while keeping their caption and legacy source metadata", () => {
    const name = "截图 (最终版) #100%.png", source = imageDocument("unicode", [name]);
    const markdown = noteDocumentToMarkdown(source), paths = imagePaths(markdown);
    expect(paths).toHaveLength(1);
    expect(paths[0]).not.toMatch(/[ ()#]/u);
    expect(paths[0]).toBe(`../assets/${"a".repeat(64)}.png`);
    expect(markdown).toContain("![unicode 图 1]");
    expect(collectNoteAssets(source)[0]?.displayName).toBe(name);
  });

  it("uses one collected filename for every alias of an asset within a note", () => {
    const source = imageDocument("aliases", ["first.png", "second (name).png"]);
    const markdown = noteDocumentToMarkdown(source), paths = imagePaths(markdown);
    expect(paths).toHaveLength(2); expect(new Set(paths).size).toBe(1);
    const chosen = collectNoteAssets(source)[0]!;
    expect(decodeURIComponent(paths[0]!)).toBe(`../assets/${chosen.id}.png`);
    expect(markdown).toContain("![aliases 图 1]"); expect(markdown).toContain("![aliases 图 2]");
    expect(source.blocks[0]?.content).toMatchObject({displayName: "first.png"});
  });

  it("still encodes attachment filenames containing spaces, parentheses and punctuation", () => {
    const source = imageDocument("attachment", ["说明 (最终) #100%.pdf"]);
    const image = source.blocks[0]!;
    const document: NoteDocumentV1 = {...source, blocks: [{...image, type: "file", content: {
      ...image.content, kind: "asset", assetId: "a".repeat(64), displayName: "说明 (最终) #100%.pdf", mimeType: "application/pdf", caption: "",
    }}]};
    const markdown = noteDocumentToMarkdown(document);
    expect(markdown).toContain("%28"); expect(markdown).toContain("%29"); expect(markdown).toContain("%23"); expect(markdown).toContain("%25");
  });

  it("matches every cross-note image alias to the single asset actually exported", async () => {
    const first = imageDocument("first", ["first screenshot.png"]), second = imageDocument("second", ["第二张 (别名)#1.png"]);
    const documents = [first, second];
    const output = vi.fn(async (request: NoteExportRequest) => ({folderName: request.folderName, fileCount: request.files.length, assetCount: request.assets.length}));
    const gateway = {listPages: async () => documents.map(doc => ({id: doc.pageId, title: doc.pageId})),
      loadDocument: async (id: string) => {const document = documents.find(doc => doc.pageId === id)!; return {page: {id, title: id}, document};},
      listOutgoing: async () => [], listReadingNotes: async () => [], exportNotes: output} as unknown as NotesGateway;
    await exportNoteLibrary(gateway, "/synthetic/export");
    const request = output.mock.calls[0]![0], paths = new Set(request.assets.map(asset => asset.relativePath));
    expect(paths.size).toBe(1); expect(request.assets).toHaveLength(1);
    for (const file of request.files) for (const path of imagePaths(file.contents)) expect(paths.has(decodeURIComponent(path).replace(/^\.\.\//u, ""))).toBe(true);
    const manifest = JSON.parse(request.manifestJson);
    expect(manifest.assets[0].relativePath).toBe(request.assets[0]!.relativePath);
    expect(manifest.documents).toEqual(documents);
    expect(request.assets[0]!.relativePath).toMatch(/\.png$/u);
  });

  it("previews Markdown and downgrades unmanaged external assets", () => {
    const preview = previewMarkdownImport(notePageId("preview"), "# Result\n\n![Plot](https://example.test/private.png)");
    expect(preview.blockCount).toBeGreaterThanOrEqual(2);
    expect(preview.warnings).toEqual([expect.stringContaining("未复制")]);
    expect(collectNoteAssets(preview.document)).toEqual([]);
    expect(JSON.stringify(preview.document)).toContain("未导入的外部资源");
  });

  it("rejects Markdown before parsing when the UTF-8 source exceeds the import budget", () => {
    const oversized = "a".repeat(MAX_MARKDOWN_IMPORT_BYTES + 1);
    expect(() => previewMarkdownImport(notePageId("page-import"), oversized)).toThrow("Markdown 文件不能超过 8 MB");
  });
});

it("round trips native collapsible lists and headings through portable details HTML", () => {
  const original = markdownToNoteDocument(notePageId("toggle"), '<details><summary><p><strong>标题 &amp; 内容</strong></p></summary><p>子段落</p><details><summary><h2>折叠标题</h2></summary><ul><li>嵌套列表</li></ul></details></details>');
  expect(original.blocks[0]).toMatchObject({type: "toggleListItem", children: [
    {type: "paragraph"}, {type: "heading", props: {level: 2, isToggleable: true}, children: [{type: "bulletListItem"}]},
  ]});
  const exported = noteDocumentToMarkdown(original);
  expect(exported).toContain("<details"); expect(exported).toContain("<strong>");
  const reopened = markdownToNoteDocument(original.pageId, exported);
  const content = (blocks: NoteDocumentV1["blocks"]): unknown => blocks.map(({type, props, content: value, children}) => ({type, props, content: value, children: content(children)}));
  expect(content(reopened.blocks)).toEqual(content(original.blocks));
});

it("preserves collapsible children inside an ordinary list during Markdown export", () => {
  const original: NoteDocumentV1 = {schemaVersion: 1, pageId: notePageId("nested-toggle"), blocks: [
    {id: noteBlockId("parent"), type: "bulletListItem", props: {}, content: {kind: "inline", items: [{type: "text", text: "Parent", styles: []}]}, children: [
      {id: noteBlockId("toggle"), type: "toggleListItem", props: {}, content: {kind: "inline", items: [{type: "text", text: "Toggle", styles: []}]}, children: [
        {id: noteBlockId("child"), type: "paragraph", props: {}, content: {kind: "inline", items: [{type: "text", text: "Child", styles: []}]}, children: []},
      ]},
    ]},
  ]};
  const reopened = markdownToNoteDocument(original.pageId, noteDocumentToMarkdown(original));
  expect(reopened.blocks).toHaveLength(1);
  expect(reopened.blocks[0]).toMatchObject({type: "bulletListItem", children: [{type: "toggleListItem", children: [{type: "paragraph"}]}]});
});
