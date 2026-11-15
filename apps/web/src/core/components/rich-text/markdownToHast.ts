import {unified} from "unified";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import remarkGfm from "remark-gfm";
import type {Root} from "hast";
import {remarkLatexMath} from "./remarkLatexMath";

// Rendering and clipboard ingress share one grammar, including block-level math.
const parser = unified().use(remarkParse).use(remarkGfm).use(remarkLatexMath)
  .use(remarkRehype, {allowDangerousHtml: true});
export function markdownToHast(value: string): Root {
  return parser.runSync(parser.parse(value)) as Root;
}
