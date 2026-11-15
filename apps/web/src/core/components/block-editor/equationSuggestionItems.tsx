import type { DefaultReactSuggestionItem } from "@blocknote/react";
import { Sigma, SquareFunction } from "lucide-react";

/** Host adapters supply schema-typed commands; every menu shares labels and behavior. */
export function buildEquationSuggestionItems(actions: {
  readonly insertInline: () => void;
  readonly insertBlock: () => void;
}): readonly [DefaultReactSuggestionItem, DefaultReactSuggestionItem] {
  return [
    {
      title: "行内公式",
      subtext: "在当前段落插入可编辑的数学公式",
      aliases: ["inline formula", "inline equation", "inline math", "latex", "数学", "公式"],
      group: "研究内容",
      badge: "$",
      icon: <Sigma size={18} />,
      onItemClick: actions.insertInline,
    },
    {
      title: "公式块",
      subtext: "插入独立显示的数学公式",
      aliases: ["formula", "equation", "latex", "数学"],
      group: "研究内容",
      badge: "$$",
      icon: <SquareFunction size={18} />,
      onItemClick: actions.insertBlock,
    },
  ];
}
