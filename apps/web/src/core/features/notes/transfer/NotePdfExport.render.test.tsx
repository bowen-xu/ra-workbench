// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { BlockNoteEditor } from "@blocknote/core";
import { notePageId } from "@research-assistant/core/notes";
import { NotesProvider } from "../NotesClient";
import { NoteEditor } from "../editor/NoteEditor";
import { blockNoteToNoteDocument } from "../editor/blockNoteAdapter";
import { noteEditorSchema, type NoteEditorInstance, type PartialNoteEditorBlock } from "../editor/noteEditorSchema";
import { noteRendererGateway } from "../../../../app/previews/hbs-adapters/noteGateway";
import { createNotePdfSnapshot } from "./notePdfSnapshot";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable:true, value:"CSS1Compat"});
});
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("prints the production note tree with its native title, image/table captions and formulas without changing its document", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const diagram = '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="260"><rect x="2" y="2" width="796" height="256" rx="16" fill="#eff4f0" stroke="#355e44" stroke-width="4"/><text x="40" y="70" font-size="32">Selection → Expansion → Simulation</text><path d="M70 150H700" stroke="#355e44" stroke-width="3"/><circle cx="150" cy="150" r="25" fill="white" stroke="#355e44"/><circle cx="400" cy="150" r="25" fill="white" stroke="#355e44"/><circle cx="650" cy="150" r="25" fill="white" stroke="#355e44"/></svg>';
  vi.spyOn(URL, "createObjectURL").mockReturnValue(`data:image/svg+xml;base64,${Buffer.from(diagram).toString("base64")}`);
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
  const gateway = new Proxy(noteRendererGateway, {get(target, key) {
    return key === "readNoteAsset" ? async () => new TextEncoder().encode(diagram) : Reflect.get(target, key);
  }});
  const blocks: PartialNoteEditorBlock[] = [
    {id:"intro", type:"paragraph", content:"这是一份合成研究笔记，用于验证中文、图表和公式的 PDF 导出。"},
    {id:"heading", type:"heading", props:{level:2}, content:"实验结果与资源说明"},
    {id:"list", type:"numberedListItem", content:"保留列表层级", children:[{id:"nested", type:"numberedListItem", content:"嵌套列表中的中文内容"}]},
    {id:"table", type:"table", props:{caption:"表 1：实验结果（中文说明）"}, content:{type:"tableContent", rows:[{cells:["方法", "准确率", "备注"]}, {cells:["MCTS", "95%", "合成测试数据"]}]}},
    {id:"image", type:"managedImage", props:{assetId:"a".repeat(64), mimeType:"image/svg+xml", caption:"图 1：搜索过程示意图", previewScale:60, textAlignment:"center"}},
    {id:"code", type:"codeBlock", props:{language:"python"}, content:'def hello():\n    return "中文结果"'},
    {id:"formula", type:"formulaBlock", props:{source:"y = w_1 \\cdot x_1 + w_2 \\cdot x_2"}},
    {id:"inline-formula", type:"paragraph", content:[{type:"text", text:"段尾行内公式：", styles:{}}, {type:"inlineFormula", props:{source:"E = mc^2"}}]},
    ...Array.from({length:18}, (_, index): PartialNoteEditorBlock => ({id:`body-${index}`, type:"paragraph", content:`第 ${index + 1} 段。用于检查跨页排版、文字完整性和段落间距。PDF 应只包含笔记内容，编辑菜单、侧边栏和工具栏不应出现在导出文件中。`})),
  ];
  const seed = BlockNoteEditor.create({schema:noteEditorSchema, initialContent:blocks}) as unknown as NoteEditorInstance;
  const doc = {...blockNoteToNoteDocument(notePageId("pdf-fixture"), seed.document), title:"笔记 PDF 导出验证"};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let editor: NoteEditorInstance | undefined;
  try {
    await act(async () => root.render(<NotesProvider gateway={gateway}><NoteEditor document={doc} pageTitle={{value:doc.title}}
      onChange={() => {}} onEditorReady={value => {editor=value;}} /></NotesProvider>));
    await vi.waitFor(() => {
      expect(host.querySelector(".note-image-block img")).not.toBeNull();
      expect(host.querySelectorAll('[data-content-type="blockCaption"]')).toHaveLength(2);
    });
    const source = host.querySelector<HTMLElement>('[data-note-pdf-page-id="pdf-fixture"]')!;
    expect(source).not.toBeNull();
    const before = JSON.stringify(editor!.document);
    const snapshot = createNotePdfSnapshot(source, doc.title);
    expect(snapshot.textContent).toContain("笔记 PDF 导出验证");
    expect(snapshot.textContent).toContain("表 1：实验结果"); expect(snapshot.textContent).toContain("图 1：搜索过程示意图");
    expect(snapshot.querySelector(".katex")).not.toBeNull();
    expect(snapshot.querySelector("img.ProseMirror-separator")).toBeNull();
    expect(snapshot.querySelector("button, .block-media-controls, .code-block-controls")).toBeNull();
    expect(JSON.stringify(editor!.document)).toBe(before);
    // Optional offline rendering artifact. It contains synthetic data only and
    // production markup/styles, never a second preview UI.
    if (process.env.NOTE_PDF_RENDER_FIXTURE_PATH) {
      const {writeFileSync, readdirSync} = await import("node:fs");
      const {resolve} = await import("node:path");
      const assets = resolve("apps/web/dist/assets");
      const links = readdirSync(assets).filter(name => name.endsWith(".css")).map(name => `<link rel="stylesheet" href="file://${assets}/${name}">`).join("");
      writeFileSync(process.env.NOTE_PDF_RENDER_FIXTURE_PATH, `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">${links}</head><body>${snapshot.outerHTML}</body></html>`);
    }
  } finally { await act(async () => root.unmount()); host.remove(); }
});
