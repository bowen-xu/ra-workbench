import { createCodeBlockSpec, type Extension } from "@blocknote/core";
import { createReactBlockSpec } from "@blocknote/react";
import { createElement } from "react";
import { blockCodeLanguages, createBlockCodeHighlightPlugin } from "./codeBlockSyntax";
import { CodeBlockView } from "./CodeBlockControls";

const upstream = createCodeBlockSpec({ supportedLanguages: blockCodeLanguages });
// The pinned React helper's types exclude plain-content blocks. Use its React
// node-view renderer only; retain upstream's plain schema, parser and export.
const reactView = createReactBlockSpec({ ...upstream.config, content: "inline" }, {
  render: ({ block, editor, contentRef }) => createElement(CodeBlockView, {
    language: block.props.language, contentRef,
    choose: language => { if (editor.isEditable) editor.updateBlock(block.id, { props: { language } }); },
    source: () => {
      const content = editor.getBlock(block.id)?.content;
      return Array.isArray(content) ? content.map(item => "text" in item ? item.text : "").join("") : "";
    },
  }),
})();
function withLocalHighlight(extension: Extension): Extension {
  return extension.key === "code-block-highlighter"
    ? { ...extension, prosemirrorPlugins: [createBlockCodeHighlightPlugin()] } : extension;
}
export const blockCodeSpec = { ...upstream,
  implementation: { ...upstream.implementation, render: reactView.implementation.render as unknown as typeof upstream.implementation.render },
  extensions: upstream.extensions?.map(extension => typeof extension === "function"
    ? (context: Parameters<typeof extension>[0]) => withLocalHighlight(extension(context)) : withLocalHighlight(extension)),
};
