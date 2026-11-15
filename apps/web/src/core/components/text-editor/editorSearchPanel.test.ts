// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { EditorView } from "@codemirror/view";
import { openSearchPanel, closeSearchPanel } from "@codemirror/search";
import { undo } from "@codemirror/commands";
import { EditorSelection } from "@codemirror/state";
import { createTextEditorState } from "./textEditorState";
let view: EditorView;
afterEach(() => { view?.destroy(); document.body.replaceChildren(); });
function setup(content: string, selection?: { from: number; to: number }, readOnly = false) {
  const host = document.createElement("div"); document.body.append(host);
  view = new EditorView({ parent: host, state: createTextEditorState({ content, path: "test.md", readOnly, onChange: () => {} }) });
  if (selection) view.dispatch({ selection: EditorSelection.range(selection.from, selection.to) });
  openSearchPanel(view);
}
function button(label: string) { return document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!; }
function field(label: string) { return document.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!; }
function type(label: string, value: string) { const input = field(label); input.value = value; input.dispatchEvent(new Event("input", { bubbles: true })); }
function key(label: string, key: string, options = {}) { field(label).dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options })); }
const status = () => document.querySelector(".editor-search-status")?.textContent;
it("uses shared hints while retaining native search buttons and their commands", async () => {
  setup("cat cat");
  const expand = button("展开替换");
  await vi.waitFor(() => expect(expand.querySelector('.editor-search-hint-anchor')).not.toBeNull());
  expect(document.querySelector('.text-search-panel [title]')).toBeNull();
  const anchor = expand.querySelector('.editor-search-hint-anchor')!;
  anchor.dispatchEvent(new MouseEvent("mouseover", {bubbles: true}));
  anchor.dispatchEvent(new MouseEvent("mousemove", {bubbles: true}));
  await vi.waitFor(() => expect(document.querySelector('[role="tooltip"]')?.textContent).toBe("展开替换"));
  expand.click();
  expect(button("收起替换")).toBe(expand);
  expect(document.querySelector<HTMLElement>('.editor-replace-row')?.hidden).toBe(false);
  type("查找内容", "cat");
  type("替换为", "dog");
  button("全部替换").click();
  expect(view.state.doc.toString()).toBe("dog dog");
});
it("shows active match, wraps navigation, and recalls history", async () => {
  setup("cat dog cat"); type("查找内容", "cat"); expect(status()).toBe("1 / 2");
  key("查找内容", "Enter"); expect(status()).toBe("2 / 2");
  key("查找内容", "Enter"); expect(status()).toBe("1 / 2");
  key("查找内容", "Enter", { shiftKey: true }); expect(status()).toBe("2 / 2");
  type("查找内容", "dog"); key("查找内容", "Enter");
  closeSearchPanel(view);
  await new Promise(resolve => setTimeout(resolve, 0));
  openSearchPanel(view); type("查找内容", "draft");
  key("查找内容", "ArrowUp"); expect(field("查找内容").value).toBe("dog");
  key("查找内容", "ArrowDown"); expect(field("查找内容").value).toBe("draft");
});
it("freezes selection scope, replaces only inside it, and supports undo", () => {
  setup("cat cat outside cat", { from: 0, to: 7 }); button("在选区中查找").click();
  type("查找内容", "cat"); expect(status()).toBe("1 / 2");
  button("展开替换").click(); type("替换为", "longer");
  key("替换为", "Enter", { metaKey: true });
  expect(view.state.doc.toString()).toBe("longer longer outside cat");
  expect(undo(view)).toBe(true); expect(view.state.doc.toString()).toBe("cat cat outside cat");
});
it("preserves upper, lower and title case and handles regex capture replacement", () => {
  setup("CAT cat Cat cAt"); type("查找内容", "cat"); button("展开替换").click();
  type("替换为", "dog"); button("保留大小写").click(); key("替换为", "Enter", { ctrlKey: true });
  expect(view.state.doc.toString()).toBe("DOG dog Dog dog"); expect(undo(view)).toBe(true);
  type("查找内容", "(c)(at)"); button("正则表达式").click(); type("替换为", "$2$1"); button("全部替换").click();
  expect(view.state.doc.toString()).toBe("ATC atc atC Atc");
});
it("uses replacement Enter, blocks invalid patterns and readonly replacement, and ignores IME Enter", () => {
  setup("one one"); type("查找内容", "one"); button("展开替换").click(); type("替换为", "two");
  key("替换为", "Enter", { isComposing: true }); expect(view.state.doc.toString()).toBe("one one");
  key("替换为", "Enter"); expect(view.state.doc.toString()).toBe("two one");
  button("正则表达式").click(); type("查找内容", "["); expect(status()).toBe("正则表达式无效"); expect(button("全部替换").disabled).toBe(true);
  view.destroy(); document.body.replaceChildren(); setup("one", undefined, true);
  expect(button("展开替换").disabled).toBe(true); type("查找内容", "one"); expect(button("替换").disabled).toBe(true);
});
it("supports option shortcuts and maps selection scope through replacements", () => {
  setup("cat CAT cat", { from: 0, to: 7 }); button("在选区中查找").click(); type("查找内容", "cat");
  key("查找内容", "c", { altKey: true, metaKey: true }); expect(button("区分大小写").getAttribute("aria-pressed")).toBe("true");
  expect(status()).toBe("1 / 1");
  button("展开替换").click(); type("替换为", "longer"); button("全部替换").click();
  expect(view.state.doc.toString()).toBe("longer CAT cat");
  type("查找内容", "CAT"); expect(status()).toBe("1 / 1"); button("全部替换").click();
  expect(view.state.doc.toString()).toBe("longer longer cat");
});
