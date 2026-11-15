import { useEffect, useRef } from "react";
import { BlockNoteSchema, defaultStyleSpecs } from "@blocknote/core";
import { createReactBlockSpec, createReactInlineContentSpec } from "@blocknote/react";
import katex from "katex";
import { TextSelection } from "@tiptap/pm/state";
import "katex/dist/katex.min.css";
import "./Formula.css";
import "./CodeBlock.css";
import { blockCodeSpec } from "./codeBlockSpec";
import { InlineFormulaEditor } from "./InlineFormulaEditor";
import { FormulaBlockEditor } from "./FormulaBlockEditor";
import { quoteBlockSpec } from "./quoteBlockSpec";
import { captionedTableSpec } from "./captionedTableSpec";
import {captionBlockSpec} from "./captionBlock";
import {noteHeadingSpec} from "./noteHeading";

function FormulaPreview({ source, displayMode }: { readonly source: string; readonly displayMode: boolean }): React.JSX.Element {
  const targetRef = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (targetRef.current === null) return;
    katex.render(source || "\\square", targetRef.current, {
      displayMode,
      throwOnError: false,
      strict: "warn",
      trust: false,
    });
  }, [displayMode, source]);
  return <span aria-label={source || "空公式"} className="note-formula-preview" ref={targetRef} />;
}

const formulaBlock = createReactBlockSpec(
  {
    type: "formulaBlock",
    propSchema: { source: { default: "" } },
    content: "none",
  },
  {
    render: function FormulaBlock({ block, editor }) {
      return <FormulaBlockEditor blockId={block.id} source={block.props.source}
        onChange={source => editor.updateBlock(block, { props: { source } })}>
        {source => <FormulaPreview displayMode source={source} />}
      </FormulaBlockEditor>;
    },
    toExternalHTML: ({block}) => <div dangerouslySetInnerHTML={{__html: katex.renderToString(block.props.source || "\\square", {
      output: "mathml", displayMode: true, throwOnError: false, strict: "warn", trust: false,
    })}} />,
  },
);

const inlineFormula = createReactInlineContentSpec(
  {
    type: "inlineFormula",
    propSchema: { source: { default: "" } },
    content: "none",
  },
  {
    render: ({ inlineContent, updateInlineContent, editor }) => <InlineFormulaEditor
      getView={() => editor.prosemirrorView}
      emptyPreview={<FormulaPreview displayMode={false} source={"\\sqrt{x}"} />}
      onChange={(source) => updateInlineContent({ type: "inlineFormula", props: { source } })}
      onDelete={element => {
        const wrapper = element.closest(".bn-inline-content-section");
        const view = editor.prosemirrorView;
        if (wrapper === null || !view.dom.contains(wrapper)) return;
        // Resolve this specific atomic node, including repeated equations in
        // one paragraph; deleting the containing block would lose nearby text.
        const position = view.posAtDOM(wrapper, 0);
        const node = view.state.doc.nodeAt(position);
        if (node?.type.name !== "inlineFormula" || String(node.attrs.source ?? "").trim() !== "") return;
        editor.transact(transaction => transaction.delete(position, position + node.nodeSize));
      }}
      onFinish={() => editor.focus()}
      onMoveOut={(element, side) => {
        const wrapper = element.closest(".bn-inline-content-section");
        const view = editor.prosemirrorView;
        if (wrapper === null || !view.dom.contains(wrapper)) return;
        const position = view.posAtDOM(wrapper, 0);
        const node = view.state.doc.nodeAt(position);
        if (node?.type.name !== "inlineFormula") return;
        editor.transact(transaction => transaction.setSelection(TextSelection.create(
          transaction.doc, side === "left" ? position : position + node.nodeSize,
        )));
      }}
      source={inlineContent.props.source}
    >{source => <FormulaPreview displayMode={false} source={source} />}</InlineFormulaEditor>,
    // Clipboard/export rendering must not mount editor controls or popovers.
    toExternalHTML: ({ inlineContent }) => <span aria-label={inlineContent.props.source || "空公式"}
      dangerouslySetInnerHTML={{ __html: katex.renderToString(inlineContent.props.source || "\\square", {
        output: "mathml", displayMode: false, throwOnError: false, strict: "warn", trust: false,
      }) }} />,
  },
);

/** Standard blocks and formulas only. Hosts extend with resource/reference specs. */
export const baseBlockEditorSchema = BlockNoteSchema.create().extend({
  styleSpecs: {
    textColor: { ...defaultStyleSpecs.textColor, implementation: { ...defaultStyleSpecs.textColor.implementation,
      // Color wraps decorations so links, underline and strike inherit it.
      runsBefore: ["link", "underline", "strike"],
    } },
  },
  blockSpecs: { heading: noteHeadingSpec, formulaBlock: formulaBlock(), codeBlock: blockCodeSpec, quote: quoteBlockSpec, table: captionedTableSpec, blockCaption: captionBlockSpec },
  inlineContentSpecs: { inlineFormula },
});
