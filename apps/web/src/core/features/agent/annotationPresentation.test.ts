import {expect, it} from "vitest";
import {annotationCommentPreview, annotationCommentPreviewParts} from "./annotationPresentation";

it("distinguishes actual line breaks from literal arrow characters", () => {
  expect(annotationCommentPreviewParts("文字 ↵ 原有箭头\n第二行")).toEqual([
    {kind:"text",text:"文字 ↵ 原有箭头 "}, {kind:"lineBreak",text:"↵"}, {kind:"text",text:" 第二行"},
  ]);
});

it("represents line breaks without changing the source comment", () => {
  const comment = "第一行\r\n第二行\n\n第三行\r第四行";
  expect(annotationCommentPreview(comment)).toBe("第一行 ↵ 第二行 ↵  ↵ 第三行 ↵ 第四行");
});
it("limits previews to 80 visible characters including the ellipsis", () => {
  expect(annotationCommentPreview("字".repeat(80))).toBe("字".repeat(80));
  expect(annotationCommentPreview("字".repeat(81))).toBe(`${"字".repeat(79)}…`);
});
it("keeps emoji and combining characters intact when truncating", () => {
  const character = "👨‍👩‍👧‍👦";
  expect(annotationCommentPreview(character.repeat(81))).toBe(`${character.repeat(79)}…`);
  expect(annotationCommentPreview("e\u0301".repeat(81))).toBe(`${"e\u0301".repeat(79)}…`);
});
