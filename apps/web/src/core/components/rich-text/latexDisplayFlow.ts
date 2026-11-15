import type { Construct, State } from "micromark-util-types";
import type { Extension } from "mdast-util-from-markdown";

declare module "micromark-util-types" {
  interface TokenTypeMap {
    researchLatexDisplay: "researchLatexDisplay";
    researchLatexDisplayValue: "researchLatexDisplayValue";
    researchLatexDisplayFence: "researchLatexDisplayFence";
  }
  interface Token { researchMathClosed?: boolean; }
}

const space = (code: number | null): boolean => code === 32 || code === -1 || code === -2;
const ending = (code: number | null): boolean => code !== null && code < -2;

/** A standalone display delimiter is a flow fence, not a text escape. Protect
 * its body before CommonMark can interpret '=', lists, HTML or thematic breaks.
 * Container continuation still belongs to micromark (including non-lazy lines). */
export const latexDisplayFlow: Construct = {
  name: "researchLatexDisplay", concrete: true,
  tokenize(effects, ok, nok) {
    const context = this;
    const token = effects.enter("researchLatexDisplay");
    const start: State = code => {effects.consume(code); return bracket;};
    const bracket: State = code => {
      if (code !== 91) return nok(code);
      effects.consume(code); return openingEnd;
    };
    const openingEnd: State = code => {
      if (space(code)) {effects.consume(code); return openingEnd;}
      if (!ending(code)) return nok(code); // same-line displays use the text construct
      if (context.interrupt) {effects.exit("researchLatexDisplay"); return ok(code);}
      return newline(code);
    };
    const finish: State = code => {effects.exit("researchLatexDisplay"); return ok(code);};
    const closed: State = code => {token.researchMathClosed = true; return finish(code);};
    const lineStart: State = code => {
      if (code === null) return finish(code);
      return effects.attempt(closingFence, closed, contentStart)(code);
    };
    const contentStart: State = code => {
      if (ending(code)) return newline(code);
      effects.enter("researchLatexDisplayValue"); return content(code);
    };
    const content: State = code => {
      if (code === null || ending(code)) {
        effects.exit("researchLatexDisplayValue");
        return code === null ? finish(code) : newline(code);
      }
      effects.consume(code); return content;
    };
    const consumeNewline: State = code => {
      effects.enter("lineEnding"); effects.consume(code); effects.exit("lineEnding");
      return lineStart;
    };
    const newline: State = code => effects.check({partial: true, tokenize(checkEffects, yes, no) {
      const after: State = next => context.parser.lazy[context.now().line] ? no(next) : yes(next);
      return next => {
        checkEffects.enter("lineEnding"); checkEffects.consume(next); checkEffects.exit("lineEnding");
        return after;
      };
    }}, consumeNewline, finish)(code);
    return start;
  },
};

const closingFence: Construct = {
  partial: true,
  tokenize(effects, ok, nok) {
    // Permit the normal 0..3-space fence indent, not an indented-code line.
    let indent = 0;
    effects.enter("researchLatexDisplayFence");
    const start: State = code => {
      if (code === 32 && indent++ < 3) {effects.consume(code); return start;}
      if (code !== 92) return nok(code);
      effects.consume(code); return bracket;
    };
    const bracket: State = code => {
      if (code !== 93) return nok(code);
      effects.consume(code); return end;
    };
    const end: State = code => {
      if (space(code)) {effects.consume(code); return end;}
      if (code !== null && !ending(code)) return nok(code);
      effects.exit("researchLatexDisplayFence");
      return ok(code);
    };
    return start;
  },
};

export const latexDisplayFromMarkdown: Extension = {
  enter: { researchLatexDisplay(token) {
    if (token.researchMathClosed) this.enter({type: "math", value: "", data: {
      hName: "code", hProperties: {className: ["language-math", "math-display"]}, hChildren: [],
    }}, token);
    else this.enter({type: "paragraph", children: [{type: "text", value: this.sliceSerialize(token)}]}, token);
    this.buffer();
  }},
  exit: {
    researchLatexDisplayValue(token) {
      this.config.enter.data!.call(this, token); this.config.exit.data!.call(this, token);
    },
    researchLatexDisplay(token) {
      const value = this.resume().trim();
      const node = this.stack[this.stack.length - 1];
      if (node?.type === "math") {
        node.value = value;
        node.data!.hChildren = [{type: "text", value}];
      }
      this.exit(token);
    },
  },
};
