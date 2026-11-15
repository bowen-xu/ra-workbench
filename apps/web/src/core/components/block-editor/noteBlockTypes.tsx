import { Code2, Heading1, Heading2, Heading3, Heading4, List, ListCollapse, ListOrdered, Pilcrow, Quote, SquareCheck } from "lucide-react";

/** The selection toolbar and block conversion menu share the same portable
 * types and explicit heading props. A regular heading must clear toggle mode. */
export const NOTE_BLOCK_TYPES = [
  { type: "paragraph", props: {}, title: "段落", Icon: Pilcrow },
  { type: "heading", props: { level: 1, isToggleable: false }, title: "一级标题", Icon: Heading1 },
  { type: "heading", props: { level: 2, isToggleable: false }, title: "二级标题", Icon: Heading2 },
  { type: "heading", props: { level: 3, isToggleable: false }, title: "三级标题", Icon: Heading3 },
  { type: "heading", props: { level: 4, isToggleable: false }, title: "四级标题", Icon: Heading4 },
  { type: "heading", props: { level: 1, isToggleable: true }, title: "可折叠一级标题", Icon: Heading1 },
  { type: "heading", props: { level: 2, isToggleable: true }, title: "可折叠二级标题", Icon: Heading2 },
  { type: "heading", props: { level: 3, isToggleable: true }, title: "可折叠三级标题", Icon: Heading3 },
  { type: "quote", props: {}, title: "引用", Icon: Quote },
  { type: "toggleListItem", props: {}, title: "可折叠列表", Icon: ListCollapse },
  { type: "bulletListItem", props: {}, title: "无序列表", Icon: List },
  { type: "numberedListItem", props: {}, title: "有序列表", Icon: ListOrdered },
  { type: "checkListItem", props: {}, title: "检查清单", Icon: SquareCheck },
  { type: "codeBlock", props: {}, title: "代码块", Icon: Code2 },
] as const;
