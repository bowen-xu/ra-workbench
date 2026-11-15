// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { BlockNoteEditor } from "@blocknote/core";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { baseBlockEditorSchema } from "./baseSchema";
import { BlockEditor } from "./BlockEditor";
import { highlightBlockCodeSnapshot } from "./codeBlockSyntax";

let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("keeps current code editable, copies live source and changes language without replacing source", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  vi.spyOn(navigator.clipboard, "writeText").mockImplementation(writeText);
  const editor = BlockNoteEditor.create({ schema: baseBlockEditorSchema, initialContent: [
    { id: "code", type: "codeBlock", props: { language: "python" }, content: 'print("hello world")' },
  ] });
  await act(async () => root.render(<BlockEditor editor={editor} onChange={() => undefined} />));
  expect(host.querySelector("code .tok-builtin")?.textContent).toBe("print");
  expect(host.querySelector("code .tok-string")?.textContent).toBe('"hello world"');
  await act(async () => editor.updateBlock("code", { content: 'print("latest")\n# comment' }));
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="复制代码"]')?.click());
  expect(writeText).toHaveBeenCalledWith('print("latest")\n# comment');
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="代码语言"]')?.click());
  const plain = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find(button => button.textContent === "纯文本");
  expect(plain).toBeDefined();
  await act(async () => plain?.click());
  const block = editor.document[0];
  expect(block?.type).toBe("codeBlock");
  if (block?.type === "codeBlock") expect(block.props.language).toBe("text");
  expect(host.querySelector("code")?.textContent).toBe('print("latest")\n# comment');
  expect(host.querySelector("code .tok-builtin")).toBeNull();
  await act(async () => root.render(<BlockEditor editor={editor} readOnly onChange={() => undefined} />));
  expect(host.querySelector<HTMLButtonElement>('[aria-label="代码语言"]')?.disabled).toBe(true);
});

it.each([
  ["python", 'if True:\n    print("hello") # comment', ".tok-builtin", "print"],
  ["javascript", "const value = 42;", ".tok-keyword", "const"],
  ["typescript", "const value: string = 'text';", ".tok-keyword", "const"],
  ["json", '{"value": 42}', ".tok-number", "42"],
  ["latex", "\\section{Title}", ".tok-tagName", "\\section"],
])("highlights %s using the same token classes for review snapshots", (language, source, selector, text) => {
  const code = document.createElement("code"); code.textContent = source;
  highlightBlockCodeSnapshot(code, language);
  expect(code.textContent).toBe(source);
  expect(code.querySelector(selector)?.textContent).toBe(text);
});
