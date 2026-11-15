import {createExtension} from "@blocknote/core";
import {closeHistory} from "@tiptap/pm/history";
import {Plugin, TextSelection} from "@tiptap/pm/state";
import type {EditorView} from "@tiptap/pm/view";

/** Toggle child boundaries have positional deletion rules; ordinary nested blocks
 * retain BlockNote's native lift command. Use its document commands/history. */
export const noteToggleEditing = createExtension(({editor}) => {
  const backspace = (view: EditorView): boolean => {
    const {selection} = view.state, {$from} = selection;
    if (!view.editable || view.composing || !(selection instanceof TextSelection) || !selection.empty
      || $from.parent.type.name !== "paragraph" || $from.parentOffset !== 0 || $from.depth < 5) return false;
    const container = $from.node($from.depth - 1);
    if (container.type.name !== "blockContainer") return false;
    const block = editor.getBlock(container.attrs.id);
    const parent = block && editor.getParentBlock(block);
    if (!block || !parent || !(parent.type === "toggleListItem" || parent.type === "heading" && parent.props.isToggleable === true)) return false;
    const index = parent.children.findIndex(child => child.id === block.id);
    if (index < 0) return false;
    const only = parent.children.length === 1;
    const last = !only && index === parent.children.length - 1;
    // Position takes precedence over content for the final row. Lifting keeps
    // its text and descendants; deletion is restricted to genuinely empty leaves.
    if (!last && ($from.parent.content.size !== 0 || container.childCount !== 1)) return false;
    const parentPos = $from.before($from.depth - 3);
    editor.transact(transaction => {
      closeHistory(transaction);
      if (last) editor.unnestBlock();
      else {
        editor.removeBlocks([block]);
        editor.setTextCursorPosition(index === 0 ? parent : parent.children[index - 1]!, "end");
      }
    });
    editor.transact(transaction => {closeHistory(transaction);});
    if (only) {
      // v0.52.1 automatically collapses after its last child disappears and has
      // no public visibility command. Invoke its own disclosure control after
      // onChange, retaining native hint rendering/local state (no shadow state).
      const dom = view.nodeDOM(parentPos);
      if (dom instanceof Element) dom.querySelector<HTMLButtonElement>(
        '.bn-toggle-wrapper[data-show-children="false"] > .bn-toggle-button',
      )?.click();
    }
    return true;
  };
  return {key: "note-toggle-editing", prosemirrorPlugins: [new Plugin({props: {
    handleDOMEvents: {
      keydown(view, event) {
        if (event.key !== "Backspace") return false;
        // Leave the browser's composition edit intact, but never send candidate
        // deletion through the editor's structural Backspace keymap.
        if (view.composing || event.isComposing || event.keyCode === 229) return true;
        if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey || !backspace(view)) return false;
        // DOM admission runs before native keymaps, including dynamically mounted
        // extensions; handleKeyDown priority alone cannot override the lift rule.
        event.preventDefault(); return true;
      },
      beforeinput(view, raw) {
        const event = raw as InputEvent;
        if (event.inputType !== "deleteContentBackward" || event.isComposing || !backspace(view)) return false;
        event.preventDefault(); return true;
      },
    },
  }})]};
});
