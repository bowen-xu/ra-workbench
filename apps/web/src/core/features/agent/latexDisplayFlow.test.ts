import {expect, it} from "vitest";
import {unified} from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import type {Root, RootContent} from "mdast";
import {remarkLatexMath} from "../../components/rich-text/remarkLatexMath";
const processor = unified().use(remarkParse).use(remarkGfm).use(remarkLatexMath);
function parse(value: string): Root {return processor.runSync(processor.parse(value));}
function nodes(root: Root | RootContent): RootContent[] {
  return [...("children" in root ? root.children.flatMap(child => nodes(child as RootContent)) : []),
    ...(root.type === "root" ? [] : [root])];
}
const formula = String.raw`\[
\operatorname{score}(r)
=
\operatorname{new\_coverage}(r)
-\lambda\,\operatorname{complexity}(r)
-\mu\,\operatorname{conflict}(r)
-\nu\,\operatorname{redundancy}(r),
\]`;
it("protects the reported standalone equality before setext heading parsing", () => {
  const tree = parse(`before\n\n${formula}\n\nafter`), all = nodes(tree);
  expect(all.filter(node => node.type === "heading")).toEqual([]);
  const math = all.filter(node => node.type === "math");
  expect(math).toHaveLength(1);
  expect(math[0]).toMatchObject({value: formula.slice(2, -2).trim()});
  expect(tree.children.map(node => node.type)).toEqual(["paragraph", "math", "paragraph"]);
});
it.each(["=", "---", "# heading", "- item", "1. item", "> quote", "<div>html</div>", "| a | b |\n| - | - |", "[id]: https://example.com", "```", "", "\\text{\\[ is literal}"])("keeps Markdown-like formula content opaque: %s", body => {
  const tree = parse(`\\[\na\n${body}\nb\n\\]`);
  expect(tree.children).toHaveLength(1);
  expect(tree.children[0]).toMatchObject({type: "math", value: `a\n${body}\nb`});
});
it.each([
  ["> \\[\n> a\n> =\n> b\n> \\]", "blockquote"],
  ["1. before\n\n   \\[\n   a\n   =\n   b\n   \\]\n\n2. after", "list"],
  ["- outer\n  - \\[\n    a\n    =\n    b\n    \\]", "list"],
])("uses native container boundaries and strips only their prefixes: %s", (value, kind) => {
  const tree = parse(value), all = nodes(tree);
  expect(tree.children[0]?.type).toBe(kind);
  expect(all.filter(node => node.type === "math")).toMatchObject([{value: "a\n=\nb"}]);
  expect(all.some(node => node.type === "heading")).toBe(false);
});
it("can interrupt a paragraph without blank lines and leaves adjacent headings intact", () => {
  const tree = parse("before\n\\[\na\n=\nb\n\\]\nafter\n\nreal heading\n===");
  expect(tree.children.map(node => node.type)).toEqual(["paragraph", "math", "paragraph", "heading"]);
});
it("accepts ordinary fence indentation and trailing whitespace", () => {
  expect(parse("  \\[ \t\na\n=\nb\n   \\] \t").children).toMatchObject([{type: "math", value: "a\n=\nb"}]);
});
it.each(["\n", "\r\n", "\r"])("preserves content across line endings %j and retains the source span", eol => {
  const source = ["\\[", "a", "=", "b", "\\]"].join(eol);
  const math = parse(source).children[0];
  expect(math).toMatchObject({type: "math", value: ["a", "=", "b"].join(eol)});
  expect(math?.position?.start.offset).toBe(0);
  expect(math?.position?.end.offset).toBe(source.length);
});
it("keeps unclosed streaming bodies literal, then compiles them when the fence closes", () => {
  for (const tail of ["", "a", "a\n=\nb", "a\n\n---\nnext"]) {
    const source = `\\[\n${tail}`;
    const all = nodes(parse(source));
    expect(all.some(node => node.type === "math" || node.type === "heading" || node.type === "thematicBreak")).toBe(false);
    expect(all.filter(node => node.type === "text").map(node => "value" in node ? node.value : "").join("")).toBe(source);
    expect(parse(`${source}\n\\]`).children[0]?.type).toBe("math");
  }
});
it("does not consume the next item or outside quote text when an unclosed container ends", () => {
  const tree = parse("> \\[\n> a\noutside\n\n- next");
  expect(tree.children.map(node => node.type)).toEqual(["blockquote", "paragraph", "list"]);
  expect(nodes(tree).filter(node => node.type === "math")).toHaveLength(0);
  expect(nodes(tree.children[0]!).filter(node => node.type === "text")).toMatchObject([{value: "\\[\na"}]);
  const list = parse("- \\[\n  a\n- next").children[0];
  expect(list).toMatchObject({type: "list", children: [{type: "listItem"}, {type: "listItem"}]});
  expect(list && nodes(list).filter(node => node.type === "math")).toHaveLength(0);
});
it("accepts every streaming prefix without leaking formula content into Markdown structure", () => {
  for (let end = 0; end <= formula.length; end++) {
    const all = nodes(parse(formula.slice(0, end)));
    expect(all.filter(node => node.type === "heading" || node.type === "list" || node.type === "thematicBreak")).toHaveLength(0);
    expect(all.filter(node => node.type === "math")).toHaveLength(end === formula.length ? 1 : 0);
  }
});
it("leaves fences, indented code, escaped delimiters and ordinary brackets literal", () => {
  for (const source of ["```latex\n\\[\na\n=\nb\n\\]\n```", "    \\[\n    a\n    \\]", String.raw`\\[escaped\\]`, "[plain]"]) {
    expect(nodes(parse(source)).filter(node => node.type === "math")).toHaveLength(0);
  }
});
it("preserves same-line display and inline math behavior", () => {
  expect(nodes(parse(String.raw`before \[a=b\] and \(r\) after`)).filter(node => node.type === "math" || node.type === "inlineMath"))
    .toMatchObject([{type: "math", value: "a=b"}, {type: "inlineMath", value: "r"}]);
});
