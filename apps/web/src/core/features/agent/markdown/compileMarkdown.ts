import { unified } from "unified";
import rehypeKatex from "rehype-katex";
import type { Root, RootContent } from "hast";
import { markdownToHast } from "../../../components/rich-text/markdownToHast";
import {sanitizeMarkdownHast} from "../../../components/rich-text/sanitizeMarkdownHast";
import { rehypeStreamReveal } from "../rehypeStreamReveal";
import { rehypeMathViewport } from "../rehypeMathViewport";
import type { MarkdownBlockPatch, MarkdownInput } from "./markdownProtocol";

// The same public unified stages used by react-markdown. Parse the whole source
// in the worker: a later reference definition can change a link in an earlier
// paragraph. Only unchanged HAST blocks may skip KaTeX/reveal and UI work.

/** Cache belongs to one visible message in one worker; release on unmount.
 * Keep only the previous snapshot, never an unbounded history of stream chunks. */
export class MarkdownCompiler {
  private previous: {signature: string; revision: number; tree: RootContent}[] = [];
  private nextRevision = 0;
  render(input: MarkdownInput): readonly MarkdownBlockPatch[] {
    const tree = markdownToHast(input.value);
    const next = tree.children.map(node => {
      const signature = JSON.stringify([input.reveal, node]);
      return {node, signature};
    });
    const patches = next.map(({node, signature}, index) => {
      const previous = this.previous[index];
      if (previous?.signature === signature) return {revision: previous.revision};
      const block: Root = {type: "root", children: [node]};
      const processor = unified().use(() => (root: Root) => rehypeStreamReveal({enabled: input.reveal, source: input.value})(root))
        .use(rehypeKatex).use(rehypeMathViewport);
      processor.runSync(block);
      sanitizeMarkdownHast(block);
      const revision = ++this.nextRevision;
      return {revision, tree: block.children[0]!};
    });
    this.previous = next.map(({signature}, index) => {
      const patch = patches[index]!;
      return {signature, revision: patch.revision, tree: patch.tree ?? this.previous[index]!.tree};
    });
    return patches;
  }
}
