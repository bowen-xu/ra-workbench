import type { Node, Slice } from "@tiptap/pm/model";

function leafText(node: Node): string {
  const text = (name: string): string => typeof node.attrs[name] === "string" ? node.attrs[name] : "";
  switch (node.type.name) {
    case "hardBreak": return "\n";
    case "inlineFormula": case "formulaBlock": return text("source");
    case "researchMention": return `@${text("labelSnapshot")}`;
    case "referenceLink":
      return [text("labelSnapshot"), text("quoteSnapshot")].filter(Boolean).join("\n");
    case "pageLink": case "childPage": return text("labelSnapshot");
    case "managedImage": case "managedFile":
      return [text("displayName"), text("caption")].filter(Boolean).join("\n");
    case "unsupportedBlock": return text("rawJson");
    case "divider": return "---";
    default: return text("caption") || text("name") || text("url");
  }
}

/** Use the actual selection slice, including partial first/last blocks. Undefined
 * leaves single-block copying (including code and table cell selections) upstream. */
export function noteClipboardText(slice: Slice): string | undefined {
  const blocks: string[] = [];
  slice.content.descendants((node) => {
    if (!node.type.isInGroup("blockContent")) return true;
    blocks.push(node.isLeaf ? leafText(node) : node.textBetween(0, node.content.size, "\n", leafText));
    return false;
  });
  return blocks.length < 2 ? undefined : blocks.filter((text) => text.trim() !== "").join("\n\n");
}
