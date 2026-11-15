import {describe, expect, it} from "vitest";
import {MarkdownCompiler} from "./compileMarkdown";
import type {MarkdownBlockPatch} from "./markdownProtocol";
const input = (value: string, reveal = false) => ({value, reveal, streaming: true});
const changed = (patches: readonly MarkdownBlockPatch[]) => patches.filter(block => block.tree);

describe("worker Markdown compilation", () => {
  it("reuses completed blocks and recompiles only the changed tail", () => {
    const compiler = new MarkdownCompiler();
    const first = compiler.render(input(String.raw`完成 \(x_i\)。` + "\n\n尾部"));
    const next = compiler.render(input(String.raw`完成 \(x_i\)。` + "\n\n尾部新增"));
    expect(first[0]?.tree).toBeDefined();
    expect(next[0]).toEqual({revision: first[0]!.revision});
    expect(changed(next)).toHaveLength(1);
    expect(JSON.stringify(first)).toContain("katex");
    expect(JSON.stringify(compiler.render(input(String.raw`完成 \(x_i\)。` + "\n\n尾部新增", true)))).toContain("stream-reveal-glyph");
  });
  it("invalidates an earlier reference link when its later definition arrives or changes", () => {
    const compiler = new MarkdownCompiler();
    compiler.render(input("[reference][target]\n\nsecond"));
    const next = compiler.render(input("[reference][target]\n\nsecond\n\n[target]: https://example.com"));
    expect(next[0]?.tree).toBeDefined();
    expect(JSON.stringify(next[0])).toContain('"href":"https://example.com"');
    const replaced = compiler.render(input("[reference][target]\n\nsecond\n\n[target]: https://other.example"));
    expect(JSON.stringify(replaced[0])).toContain("https://other.example");
  });
  it.each([
    ["```js\ncode", "```js\ncode\n```\n\nnormal"],
    [String.raw`before \(x_i`, String.raw`before \(x_i\) after`],
    ["|a|b|\n|-|-|\n|1", "|a|b|\n|-|-|\n|1|2|"],
    ["1. first", "1. first\n2. next\n   continued"],
  ])("matches a fresh full parse after completing syntax: %s", (before, after) => {
    const compiler = new MarkdownCompiler();
    compiler.render(input(before));
    const result = compiler.render(input(after));
    const fresh = new MarkdownCompiler().render(input(after));
    expect(result.map(block => block.tree)).toEqual(fresh.map(block => block.tree));
  });
  it("preserves the existing raw HTML and URL security policy", () => {
    const result = new MarkdownCompiler().render(input('<script>alert(1)</script>\n\n[x](javascript:alert) ![image](data:text/html,boom) [note](source://notes/id)'));
    const json = JSON.stringify(result);
    expect(json).not.toContain('"tagName":"script"');
    expect(json).toContain('<script>alert(1)</script>');
    expect(json).not.toContain('"href":"javascript:');
    expect(json).not.toContain('"src":"data:');
    expect(json).toContain('"href":"source://notes/id"');
  });
});
