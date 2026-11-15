import { describe, expect, it } from "vitest";
import { Schema } from "@tiptap/pm/model";
import { EditorState, TextSelection } from "@tiptap/pm/state";
import { canOpenSuggestionMenu, isSuggestionQueryAllowed } from "./suggestionTriggerPolicy";

const schema = new Schema({
  nodes: {
    doc: { content: "block+" },
    paragraph: { group: "block", content: "inline*" },
    codeBlock: { group: "block", content: "text*", code: true },
    text: { group: "inline" },
    inlineFormula: { group: "inline", inline: true, atom: true },
  },
  marks: { code: { code: true }, bold: {} },
});
function transaction(prefix: string, block = "paragraph", codeMark = false) {
  const content = prefix ? schema.text(prefix) : undefined;
  const doc = schema.node("doc", null, [schema.node(block, null, content)]);
  let tr = EditorState.create({ schema, doc, selection: TextSelection.create(doc, prefix.length + 1) }).tr;
  if (codeMark) tr = tr.setStoredMarks([schema.marks.code!.create()]);
  return tr;
}

describe("shared typed suggestion admission", () => {
  it.each(["", " ", "   ", "text ", "中文 ", "  x   ", "text\u3000", "\t", "\u3000", "\u00a0"])("accepts block start or a preceding space: %j", prefix => {
    expect(canOpenSuggestionMenu(transaction(prefix))).toBe(true);
  });
  it.each(["text", "中文", " /", "$", "  x", "\u200b"])("rejects an adjacent non-space character: %j", prefix => {
    expect(canOpenSuggestionMenu(transaction(prefix))).toBe(false);
  });
  it.each(["text", "中文", "：", "x=", "$"])("allows dollar formulas directly after %j without changing slash/reference boundaries", prefix => {
    expect(canOpenSuggestionMenu(transaction(prefix), "$")).toBe(true);
    for (const trigger of ["/", "@"]) expect(canOpenSuggestionMenu(transaction(prefix), trigger)).toBe(false);
  });
  it("keeps slash/reference boundaries after inline atoms while allowing adjacent dollar formulas", () => {
    const doc = schema.node("doc", null, [schema.node("paragraph", null, [schema.node("inlineFormula"), schema.text(" ")])]);
    const state = EditorState.create({ schema, doc });
    expect(canOpenSuggestionMenu(state.tr.setSelection(TextSelection.create(doc, 2)))).toBe(false);
    expect(canOpenSuggestionMenu(state.tr.setSelection(TextSelection.create(doc, 2)), "$")).toBe(true);
    expect(canOpenSuggestionMenu(state.tr.setSelection(TextSelection.create(doc, 3)))).toBe(true);
  });
  it("checks the current block, not previous paragraphs or later text", () => {
    const doc = schema.node("doc", null, [schema.node("paragraph", null, schema.text("Previous")), schema.node("paragraph", null, schema.text("  Later"))]);
    const pos = doc.child(0).nodeSize + 3;
    expect(canOpenSuggestionMenu(EditorState.create({ schema, doc, selection: TextSelection.create(doc, pos) }).tr)).toBe(true);
  });
  it("keeps code and replacements outside typed suggestions", () => {
    expect(canOpenSuggestionMenu(transaction("", "codeBlock"))).toBe(false);
    expect(canOpenSuggestionMenu(transaction("", "paragraph", true))).toBe(false);
    const tr = transaction("word");
    tr.setSelection(TextSelection.create(tr.doc, 1, 5));
    expect(canOpenSuggestionMenu(tr)).toBe(false);
    expect(canOpenSuggestionMenu(tr, "$")).toBe(false);
    expect(canOpenSuggestionMenu(transaction("x", "codeBlock"), "$")).toBe(false);
    expect(canOpenSuggestionMenu(transaction("x", "paragraph", true), "$")).toBe(false);
  });
  it("retains dollar switching and slash query limits independently of admission", () => {
    for (const query of ["", "$"]) expect(isSuggestionQueryAllowed("$", query)).toBe(true);
    for (const query of [" ", "100", "$$"]) expect(isSuggestionQueryAllowed("$", query)).toBe(false);
    expect(isSuggestionQueryAllowed("/", "inline equation")).toBe(true);
    expect(isSuggestionQueryAllowed("/", "inline equation ")).toBe(false);
    expect(isSuggestionQueryAllowed("@", "A research target")).toBe(true);
  });
});
