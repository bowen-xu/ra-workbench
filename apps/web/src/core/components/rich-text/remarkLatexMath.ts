import type { Root, RootContent } from "mdast";
import type { Extension as FromMarkdownExtension } from "mdast-util-from-markdown";
import type { Construct, State, Extension as SyntaxExtension } from "micromark-util-types";
import type { Plugin } from "unified";
import type {} from "mdast-util-math";
import { latexDisplayFlow, latexDisplayFromMarkdown } from "./latexDisplayFlow";

declare module "micromark-util-types" {
  interface TokenTypeMap { researchLatexMath: "researchLatexMath"; }
}
declare module "unified" {
  interface Data {
    micromarkExtensions?: SyntaxExtension[];
    fromMarkdownExtensions?: Array<FromMarkdownExtension | FromMarkdownExtension[]>;
  }
}

// A parser extension runs before CommonMark escapes, preserving source positions
// and leaving code spans, fences, link destinations and escaped backslashes alone.
const construct: Construct = {
  name: "researchLatexMath",
  tokenize(effects, ok, nok) {
    let close: number;
    const start: State = code => {
      effects.enter("researchLatexMath");
      effects.consume(code);
      return opening;
    };
    const opening: State = code => {
      if (code !== 40 && code !== 91) return nok(code);
      close = code === 40 ? 41 : 93;
      effects.consume(code);
      return body;
    };
    const body: State = code => {
      if (code === null) return nok(code);
      if (code < -2) {
        effects.enter("lineEnding");
        effects.consume(code);
        effects.exit("lineEnding");
        return body;
      }
      effects.enter("data");
      return data(code);
    };
    const data: State = code => {
      if (code === null) return nok(code);
      if (code < -2) {
        effects.exit("data");
        return body(code);
      }
      effects.consume(code);
      return code === 92 ? afterSlash : data;
    };
    const afterSlash: State = code => {
      if (code === null) return nok(code);
      if (code < -2) {
        effects.exit("data");
        return body(code);
      }
      effects.consume(code);
      if (code === close) {
        effects.exit("data");
        effects.exit("researchLatexMath");
        return ok;
      }
      return data;
    };
    return start;
  },
};

const fromMarkdown: FromMarkdownExtension = {
  enter: {
    researchLatexMath(token) {
      const raw = this.sliceSerialize(token);
      const display = raw.startsWith("\\[");
      const value = raw.slice(2, -2).trim();
      this.enter({
        type: display ? "math" : "inlineMath", value,
        data: {
          hName: "code",
          hProperties: { className: ["language-math", display ? "math-display" : "math-inline"] },
          hChildren: [{ type: "text", value }],
        },
      }, token);
      this.buffer();
    },
  },
  exit: { researchLatexMath(token) { this.resume(); this.exit(token); } },
};

export const remarkLatexMath: Plugin<[], Root> = function () {
  const data = this.data();
  (data.micromarkExtensions ??= []).push({ flow: {92: latexDisplayFlow}, text: { 92: construct } });
  (data.fromMarkdownExtensions ??= []).push(fromMarkdown, latexDisplayFromMarkdown);
  return tree => {
    function lift(parent: { children: RootContent[] }): void {
      parent.children = parent.children.map(node => {
        if (node.type === "paragraph" && node.children.length === 1) {
          // The inline tokenizer can also consume a multiline display formula.
          const child = node.children[0] as RootContent;
          if (child.type === "math") return child;
        }
        if ("children" in node) lift(node as { children: RootContent[] });
        return node;
      });
    }
    lift(tree);
  };
};
