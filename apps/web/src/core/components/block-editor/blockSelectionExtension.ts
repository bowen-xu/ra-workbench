import type { Extension } from "@blocknote/core";
import { Plugin, PluginKey, Selection } from "@tiptap/pm/state";
import { BlockSetSelection } from "./BlockSetSelection";
import { closeHistory, isHistoryTransaction } from "@tiptap/pm/history";

/** Keep a structural selection when focus synchronizes its DOM endpoints.
 * Native typing, paste, cut and deletion then use the same engine selection.
 */
export function blockSelectionExtension(indent: (ids: readonly string[], direction: "indent" | "outdent") => boolean): Extension {
  return { key: "researchBlockSelection", prosemirrorPlugins: [new Plugin({
    key: new PluginKey("researchBlockSelection"),
    appendTransaction(transactions, previous, current) {
      // Undoing an earlier creation may remove one member of a later explicit
      // selection. Preserve its surviving roots; explicit historical structural
      // selections (e.g. undoing Delete) already carry the right bookmark.
      if (!(previous.selection instanceof BlockSetSelection)
        || current.selection instanceof BlockSetSelection || !transactions.some(isHistoryTransaction)) return null;
      const selection = BlockSetSelection.create(current.doc, previous.selection.blockIds, current.selection.anchor);
      return selection instanceof BlockSetSelection ? current.tr.setSelection(selection).setMeta("addToHistory", false) : null;
    },
    props: {
      handleDOMEvents: {
        keydown(view, event) {
          const selection = view.state.selection;
          if (!view.editable || !(selection instanceof BlockSetSelection) || event.key !== "Tab"
            || event.ctrlKey || event.metaKey || event.altKey || view.composing || event.isComposing || event.keyCode === 229) return false;
          event.preventDefault();
          view.dispatch(closeHistory(view.state.tr));
          if (indent(selection.blockIds, event.shiftKey ? "outdent" : "indent")) {
            view.dispatch(view.state.tr.setSelection(BlockSetSelection.create(view.state.doc, selection.blockIds, selection.anchor))
              .setMeta("addToHistory", false));
          }
          // A blocked whole-group move must not fall through to a native
          // continuous-range indent or browser focus traversal.
          return true;
        },
        beforeinput(view, event) {
          if (!view.editable || !(view.state.selection instanceof BlockSetSelection)) return false;
          // A DOM Range can only cover one continuous interval. Browser-driven
          // input must consume the engine's possibly disjoint ranges first;
          // letting the browser replace its DOM Range would erase unselected gaps.
          if (event.inputType === "insertText" || event.inputType === "insertReplacementText") {
            if (event.data === null) return false;
            event.preventDefault();
            view.dispatch(view.state.tr.insertText(event.data).scrollIntoView());
            return true;
          }
          if (event.inputType.startsWith("insert") || event.inputType.startsWith("delete")) {
            view.dispatch(view.state.tr.deleteSelection().scrollIntoView());
            if (event.inputType.startsWith("delete")) { event.preventDefault(); return true; }
          }
          return false;
        },
        compositionstart(view) {
          // Establish a legal insertion point before the IME owns native DOM
          // mutation. The engine's history groups replacement with composition.
          if (view.editable && view.state.selection instanceof BlockSetSelection) {
            const start = view.state.selection.from;
            const paragraph = view.state.schema.nodes.paragraph!.createAndFill()!;
            const transaction = view.state.tr.replaceSelectionWith(paragraph);
            transaction.setSelection(Selection.near(transaction.doc.resolve(start)));
            view.dispatch(transaction.scrollIntoView());
          }
          return false;
        },
      },
      createSelectionBetween(view, anchor, head) {
        const selection = view.state.selection;
        return selection instanceof BlockSetSelection
          && (view.dom.closest<HTMLElement>(".block-editor")?.dataset.blockSelecting === "true"
            || anchor.pos === selection.anchor && head.pos === selection.head) ? selection : null;
      },
    },
  })] };
}
