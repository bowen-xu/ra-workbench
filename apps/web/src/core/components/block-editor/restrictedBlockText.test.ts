// @vitest-environment happy-dom
import {DOMParser, DOMSerializer, Fragment, Schema} from "@tiptap/pm/model";
import {expect, it} from "vitest";
import {legalRestrictedText, normalizeRestrictedText, restrictedTextNodeSpec} from "./restrictedBlockText";

it("validates constrained edits without trimming, including a zero-length limit", () => {
  expect(legalRestrictedText("标题", {maxLength: 2, singleLine: true})).toBe(true);
  expect(legalRestrictedText("标题长", {maxLength: 2})).toBe(false);
  expect(legalRestrictedText("标题\n正文", {singleLine: true})).toBe(false);
  expect(legalRestrictedText("标题\r正文", {singleLine: true})).toBe(false);
  expect(legalRestrictedText("说明\n第二行", {maxLength: 10})).toBe(true);
  expect(legalRestrictedText("", {maxLength: 0})).toBe(true);
  expect(legalRestrictedText("x", {maxLength: 0})).toBe(false);
});

it("normalizes initial single-line text and preserves UTF-16 character boundaries", () => {
  expect(normalizeRestrictedText("一\r\n二\n三", {singleLine: true})).toBe("一 二 三");
  expect(normalizeRestrictedText("ab😀cd", {maxLength: 3})).toBe("ab");
  expect(normalizeRestrictedText("ab😀cd", {maxLength: 4})).toBe("ab😀");
  expect(normalizeRestrictedText("x", {maxLength: 0})).toBe("");
  expect(normalizeRestrictedText("  说明\n第二行  ")).toBe("  说明\n第二行  ");
});

it("creates a native multiline text node that retains whitespace and rejects formatting marks", () => {
  const field = restrictedTextNodeSpec({tag: "p", className: "test-caption", label: "图片说明", multiline: true});
  const schema = new Schema({nodes: {doc: {content: "field"}, field, text: {}}, marks: {bold: {}}});
  const text = "说明  保留空格\n第二行";
  const doc = schema.node("doc", null, schema.node("field", null, schema.text(text)));
  expect(schema.nodes.field!.spec.code).toBe(true);
  expect(schema.nodes.field!.spec.whitespace).toBe("pre");
  expect(schema.nodes.field!.validContent(Fragment.from(schema.text("粗体", [schema.mark("bold")])))).toBe(false);
  const host = document.createElement("div");
  host.append(DOMSerializer.fromSchema(schema).serializeFragment(doc.content));
  expect(host.querySelector("p")?.className).toBe("test-caption");
  expect(host.querySelector("p")?.getAttribute("aria-label")).toBe("图片说明");
  expect(DOMParser.fromSchema(schema).parse(host).textContent).toBe(text);
});

it("provides the same plain-text DOM hole for a single-line title without enabling code semantics", () => {
  const field = restrictedTextNodeSpec({tag: "h1", className: "document-title", label: "笔记标题"});
  expect(field.content).toBe("text*"); expect(field.marks).toBe(""); expect(field.code).toBe(false);
  expect(field.toDOM()).toEqual(["h1", {class: "document-title", "aria-label": "笔记标题", "data-restricted-block-text": "true"}, 0]);
});
