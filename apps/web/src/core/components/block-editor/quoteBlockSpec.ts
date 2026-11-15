import { createQuoteBlockSpec, type Extension } from "@blocknote/core";

const upstreamQuote = createQuoteBlockSpec();

function removeMarkdownQuoteInput(extension: Extension): Extension {
  return {
    ...extension,
    // In portable notes, `>` is reserved for a future lossless toggle block.
    inputRules: extension.inputRules?.filter((rule) => !rule.find.test("> ")) ?? [],
  };
}

/** Keep upstream quote rendering/import, but unregister its Markdown `>` rule. */
export const quoteBlockSpec = {
  ...upstreamQuote,
  extensions: upstreamQuote.extensions?.map((extension) => typeof extension === "function"
    ? (context: Parameters<typeof extension>[0]) => removeMarkdownQuoteInput(extension(context))
    : removeMarkdownQuoteInput(extension)),
};
