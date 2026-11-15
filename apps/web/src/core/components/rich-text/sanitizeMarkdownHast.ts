import type {Root, Element} from "hast";
import {defaultUrlTransform} from "react-markdown";
import {urlAttributes} from "html-url-attributes";

/** Shared renderer/import boundary: user HTML is literal; source links keep
 * their existing application protocol and all other URLs use the standard filter. */
export function sanitizeMarkdownHast(tree: Root): void {
  function visit(parent: Root | Element): void {
    parent.children = parent.children.map(node => {
      if (node.type === "raw") return {type: "text", value: node.value};
      if (node.type === "element") {
        for (const [key, tags] of Object.entries(urlAttributes)) {
          if (Object.hasOwn(node.properties, key) && (tags === null || tags.includes(node.tagName))) {
            const value = String(node.properties[key] ?? "");
            node.properties[key] = value.startsWith("source://") ? value : defaultUrlTransform(value);
          }
        }
        visit(node);
      }
      return node;
    });
  }
  visit(tree);
}
