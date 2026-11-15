// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { createNotePdfSnapshot } from "./notePdfSnapshot";

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function source(html: string): HTMLElement {
  const root = document.createElement("div"); root.className = "bn-editor"; root.innerHTML = html;
  root.dataset.notePdfPageId = "page";
  vi.spyOn(root, "getBoundingClientRect").mockReturnValue({width: 600} as DOMRect);
  return root;
}

it("exports candidate nodes while excluding review-only content, controls and selection, without editing the source", () => {
  const original = source(`<div class="bn-block-content ProseMirror-selectednode note-diff-added-block" contenteditable="true" id="live">
    <span class="note-diff-added-text">当前中文正文</span><span class="ProseMirror-widget">旧正文</span>
    <div class="note-diff-deleted-content">旧代码</div><button>编辑</button>
    <table><tbody><tr class="hbs-table-row-removed"><td>旧单元格</td></tr><tr><td>现有单元格</td></tr></tbody></table>
    <pre><code>print('完整源码')</code></pre><button class="note-formula-disclosure"><span class="katex">公式</span></button>
    <button class="note-page-link-block" role="link"><strong>关联笔记标题</strong></button></div>`);
  const before = original.outerHTML;
  const result = createNotePdfSnapshot(original, "标题 <script>");
  expect(result.textContent).toContain("当前中文正文");
  expect(result.textContent).toContain("现有单元格");
  expect(result.textContent).toContain("完整源码");
  expect(result.querySelector(".katex")).not.toBeNull();
  expect(result.textContent).toContain("关联笔记标题");
  expect(result.textContent).not.toMatch(/旧正文|旧代码|旧单元格|编辑/);
  expect(result.querySelector("[contenteditable], [id], .ProseMirror-selectednode, .note-diff-added-text")).toBeNull();
  expect(result.querySelector("h1")?.textContent).toBe("标题 <script>");
  expect(result.querySelector("script")).toBeNull();
  expect(original.outerHTML).toBe(before);
});

it("keeps one live document title and omits an empty active caption", () => {
  const original = source('<div data-content-type="documentTitle">当前标题</div><div class="block-caption-container"><div data-content-type="blockCaption"><br class="ProseMirror-trailingBreak"></div></div>');
  const result = createNotePdfSnapshot(original, "旧标题");
  expect(result.querySelector("h1")).toBeNull();
  expect(result.textContent).not.toContain("旧标题");
  expect(result.querySelector(".block-caption-container")).toBeNull();
});

it("omits editor cursor helper images while retaining inline content", async () => {
  // ProseMirror adds a source-less IMG after noneditable inline nodes on
  // Safari/Chrome. WebKit's img.decode() rejects it with Missing source URL.
  const original = source('<p><span contenteditable="false" class="katex">E = mc²</span><img class="ProseMirror-separator" alt=""><br class="ProseMirror-trailingBreak"></p>');
  const before = original.outerHTML;
  const result = createNotePdfSnapshot(original, "合成笔记");
  expect(result.querySelector(".katex")?.textContent).toBe("E = mc²");
  expect(result.querySelector("img.ProseMirror-separator")).toBeNull();
  expect(original.outerHTML).toBe(before);
});

it("preserves image scale, alignment and caption spacing on paper rather than screen pixels", () => {
  const original = source(`<div class="bn-block"><figure class="note-image-block" data-scale="40" data-alignment="center">
    <div class="note-image-resizable" style="width:240px"><div class="note-image-block-media"><img src="data:image/png;base64,AA=="></div></div></figure>
    <div class="block-caption-container"><div data-content-type="blockCaption" style="width:240px;margin-left:180px;margin-top:2px">图片说明</div></div></div>`);
  const result = createNotePdfSnapshot(original, "标题");
  expect(result.querySelector<HTMLElement>("figure")!.style.gridTemplateColumns).toBe("40%");
  expect(result.querySelector<HTMLElement>(".note-image-resizable")!.style.width).toBe("100%");
  const caption = result.querySelector<HTMLElement>('[data-content-type="blockCaption"]')!;
  expect(caption.style.width).toBe("40%"); expect(caption.style.marginLeft).toBe("auto"); expect(caption.style.marginRight).toBe("auto"); expect(caption.style.marginTop).toBe("2px");
});

it("scales a wide table to the paper width and keeps nonempty captions", () => {
  const result = createNotePdfSnapshot(source(`<div class="bn-block"><table><colgroup><col width="800"><col width="400"></colgroup><tbody><tr><td>A</td><td>B</td></tr></tbody></table>
    <div class="block-caption-container"><div data-content-type="blockCaption" style="width:1200px;margin-left:4px;margin-top:-15px">表格说明</div></div></div>`), "标题");
  expect(result.querySelector("table")!.style.width).toBe("100%");
  expect(result.querySelector("col")!.style.width).toContain("66.666");
  expect(result.querySelector<HTMLElement>('[data-content-type="blockCaption"]')!.style.marginTop).toBe("10px");
});
