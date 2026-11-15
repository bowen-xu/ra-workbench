// @vitest-environment happy-dom
import {afterEach, expect, it} from "vitest";
import katex from "katex";
import {selectionClipboard} from "./selectionClipboard";
import {markdownFormulaHTML, normalizeFormulaHTML} from "./formulaClipboard";
import {BlockNoteEditor} from "@blocknote/core";
import {baseBlockEditorSchema} from "../block-editor/baseSchema";
import {markdownToHast} from "./markdownToHast";
function host(html: string): HTMLDivElement {const node=document.createElement("div");node.innerHTML=html;document.body.append(node);return node;}
function all(node: HTMLElement): Range {const range=document.createRange();range.selectNodeContents(node);return range;}
afterEach(()=>document.body.replaceChildren());
it("keeps Chinese punctuation-adjacent emphasis readable as Markdown",()=>{
 const root=host("<p><strong>核心问题：</strong>如何压缩？<em>提示：</em>继续。</p>");
 const copy=selectionClipboard(root,all(root)),rendered=JSON.stringify(markdownToHast(copy.text));
 expect(rendered).toContain('"tagName":"strong"');
 expect(rendered).toContain('"tagName":"em"');
});
it("copies one formula as Markdown and one MathML tree without visible/accessible duplicates", () => {
 const formula=katex.renderToString("x_i=1",{displayMode:true});
 const root=host(`<p>评分：</p><span class="agent-math-viewport" data-math-display="true">${formula}</span><p>其中 <span class="agent-math-viewport" data-math-display="false">${katex.renderToString("r")}</span> 是候选。</p>`);
 const copy=selectionClipboard(root,all(root));
 expect(copy.text).toContain("\\[\nx_i=1\n\\]");
 expect(copy.text).toContain("其中 \\(r\\) 是候选。");
 const html=host(copy.html);
 expect(html.querySelectorAll("math")).toHaveLength(2);
 expect(html.querySelector(".katex-html, .katex-mathml")).toBeNull();
 expect(html.querySelector('math[display="block"]')).not.toBeNull();
 expect(html.querySelector('[data-source="x_i=1"]')).not.toBeNull();
 expect(copy.text).not.toContain("其中 rr");
});
it("retains formatting of a partial selection, expands partial equations and excludes controls",()=>{
 const root=host(`<p>前 <strong>加粗文本</strong> 后 <span class="agent-math-viewport" data-math-display="false">${katex.renderToString("x^2")}</span> 结束</p><button>复制</button>`);
 const range=document.createRange(),text=root.querySelector("strong")!.firstChild!;
 range.setStart(text,1);range.setEnd(text,3);
 expect(selectionClipboard(root,range).text).toBe("**粗文**");
 range.setStart(root.querySelector('.katex-html .mord')!.firstChild!,0);
 range.setEnd(root.querySelector('.katex-html .mord')!.firstChild!,1);
 expect(selectionClipboard(root,range).text).toBe("\\(x^2\\)");
 expect(selectionClipboard(root,all(root)).text).not.toContain("复制");
});
it("retains lists, links, tables and code as Markdown and semantic HTML",()=>{
 const root=host('<h2>标题</h2><ol start="3"><li><strong>强调</strong><ul><li><a href="https://example.com">链接</a></li></ul></li></ol><table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table><pre><code class="language-js">const x = 1;\n</code></pre>');
 const copy=selectionClipboard(root,all(root));
 expect(copy.text).toContain("## 标题");expect(copy.text).toContain("3. **强调**");
 expect(copy.text).toContain("[链接](https://example.com)");expect(copy.text).toContain("| A | B |");
 expect(copy.text).toContain("```js\nconst x = 1;\n```");
 expect(copy.html).toContain('<ol start="3">');
});
it("imports Markdown with shared math syntax into native editable formula nodes",()=>{
 const source="# 标题\n\n文字 \\(r\\)。\n\n\\[\na\n=\nb\n\\]\n\n**后文**";
 const html=markdownFormulaHTML(source)!;
 const editor=BlockNoteEditor.create({schema:baseBlockEditorSchema});
 const blocks=editor.tryParseHTMLToBlocks(html);
 expect(blocks.map(block=>block.type)).toEqual(["heading","paragraph","formulaBlock","paragraph"]);
 expect(blocks[2]!.props).toMatchObject({source:"a\n=\nb"});
 expect(blocks[1]!.content).toEqual(expect.arrayContaining([expect.objectContaining({type:"inlineFormula",props:{source:"r"}})]));
});
it("imports copied rich MathML to the same native nodes and keeps source once",()=>{
 const root=host(`<p>行内 <span class="agent-math-viewport" data-math-display="false">${katex.renderToString("r")}</span> 后文。</p><span class="agent-math-viewport" data-math-display="true">${katex.renderToString("a=b",{displayMode:true})}</span>`);
 const copy=selectionClipboard(root,all(root)),html=normalizeFormulaHTML(copy.html,document)!;
 const editor=BlockNoteEditor.create({schema:baseBlockEditorSchema}),blocks=editor.tryParseHTMLToBlocks(html);
 expect(blocks.map(block=>block.type)).toEqual(["paragraph","formulaBlock"]);
 expect(blocks[1]!.props).toMatchObject({source:"a=b"});
 expect(blocks[0]!.content).toEqual(expect.arrayContaining([expect.objectContaining({type:"inlineFormula",props:{source:"r"}})]));
});
it("does not reinterpret code or raw HTML as math, and returns null when no math exists",()=>{
 expect(markdownFormulaHTML('`\\(x\\)`\n\n```latex\n\\[\na\n=\nb\n\\]\n```')).toBeNull();
 expect(markdownFormulaHTML("ordinary **Markdown**")).toBeNull();
 const html=markdownFormulaHTML('<img src=x onerror="alert(1)">\n\n\\(x\\)')!;
 expect(html).not.toContain('<img');expect(host(html).textContent).toContain('<img');
 expect(normalizeFormulaHTML('<p>ordinary</p>',document)).toBeNull();
 const unsafe=markdownFormulaHTML("[unsafe](javascript:alert) \\(x\\)")!;
 expect(unsafe).not.toContain('href="javascript:');
});
it("exports native note equations as semantic MathML without editor controls or duplicate glyphs",()=>{
 const editor=BlockNoteEditor.create({schema:baseBlockEditorSchema});
 const html=editor.blocksToHTMLLossy([
  {type:"paragraph",content:[{type:"text",text:"before ",styles:{}},{type:"inlineFormula",props:{source:"x_i"}},{type:"text",text:" after",styles:{}}]},
  {type:"formulaBlock",props:{source:"a=b"}},
 ]);
 const fragment=host(html);
 expect(fragment.querySelectorAll("math")).toHaveLength(2);
 expect(fragment.querySelector("button, .katex-html")).toBeNull();
 const restored=editor.tryParseHTMLToBlocks(normalizeFormulaHTML(html,document)!);
 expect(restored[1]).toMatchObject({type:"formulaBlock",props:{source:"a=b"}});
});
