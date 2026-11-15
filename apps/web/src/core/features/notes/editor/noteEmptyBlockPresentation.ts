import { createExtension } from "@blocknote/core";
import { Plugin } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

/** Presentation-only decorations: empty blocks remain in the document/selection. */
export const noteEmptyBlockPresentation = createExtension(() => ({
  key: "note-empty-block-presentation",
  prosemirrorPlugins: [new Plugin({
    props: {
      decorations(state) {
        const empty: Decoration[] = [];
        state.doc.descendants((node, position) => {
          if (node.type.isInGroup("blockContent") && node.isTextblock && node.content.size === 0) {
            empty.push(Decoration.node(position, position + node.nodeSize, { "data-note-empty-block": "true" }));
          }
        });
        return DecorationSet.create(state.doc, empty);
      },
    },
  })],
}));
