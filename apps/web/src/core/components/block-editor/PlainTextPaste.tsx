import {useEffect} from "react";
import {useBlockNoteEditor, useEditorState} from "@blocknote/react";
import {Plugin} from "@tiptap/pm/state";
import {closeHistory} from "@tiptap/pm/history";
import {readClipboardText} from "../../runtime/platformClipboard";
import {notify} from "../../services/notifications";

/** Own the gesture on every platform: WebKit need not assign Cmd+Shift+V a
 * native paste command. Clipboard reading is the platform edge; insertion uses
 * ProseMirror's plain-text paste, selection, constraints and history. Never
 * route this text through BlockNote's HTML/Markdown/file paste handler. */
export function PlainTextPaste(): null {
  const editor = useBlockNoteEditor();
  const root = useEditorState({selector: context => context.editor?.domElement});
  useEffect(() => {
    const host = root?.closest(".block-editor");
    if (!root || !host) return;
    let mounted = true, request = 0;
    let rawPaste: ClipboardEvent | undefined;
    const extension = {key:"plain-text-paste", prosemirrorPlugins:[new Plugin({props:{
      handlePaste(view, event, slice) {
        if (event !== rawPaste) return false;
        // view.pasteText parses literal text, but its default uiEvent="paste"
        // then activates Tiptap's Markdown paste rules. Consume the parsed Slice
        // through native Selection.replace before that stage. No parser fork,
        // synthetic DOM paste, global rule toggle or second history is involved.
        view.dispatch(closeHistory(view.state.tr.replaceSelection(slice))
          .setMeta("paste",true).setMeta("uiEvent","paste-plain").setMeta("preventAutolink",true).scrollIntoView());
        return true;
      },
    }})]};
    editor.registerExtension(extension);
    const keyDown = (event: Event) => {
      if (!(event instanceof KeyboardEvent) || event.key.toLowerCase() !== "v" || !event.shiftKey
        || !(event.metaKey || event.ctrlKey) || event.altKey || event.isComposing || event.keyCode === 229
        || editor.prosemirrorView.composing || !(event.target instanceof Element) || !root.contains(event.target)
        || event.target.closest('input, textarea, button, select, [contenteditable="false"]')) return;
      event.preventDefault(); event.stopImmediatePropagation();
      if (event.repeat || !editor.isEditable) return;
      const id = ++request, view = editor.prosemirrorView;
      const {doc, selection} = view.state;
      // Start the read in the trusted key handler, preserving browser activation.
      // A late reply must never replace a later edit, selection or another note.
      void readClipboardText().then(text => {
        if (!mounted || id !== request || view.isDestroyed || !editor.isEditable) return;
        if (!view.hasFocus() || !view.state.doc.eq(doc) || !view.state.selection.eq(selection)) {
          notify.info("NOTE_PASTE_CANCELLED：编辑位置已改变，纯文本粘贴已取消。");
          return;
        }
        if (text) {
          const data = new DataTransfer(); data.setData("text/plain",text);
          rawPaste = new ClipboardEvent("paste",{clipboardData:data});
          try {view.pasteText(text,rawPaste);} finally {rawPaste=undefined;}
        }
      }).catch(cause => {
        if (mounted && id === request) notify.error(`NOTE_PASTE_TEXT_FAILED：${cause instanceof Error ? cause.message : String(cause)}`);
      });
    };
    host.addEventListener("keydown", keyDown, true);
    return () => {mounted = false; request++; host.removeEventListener("keydown", keyDown, true); editor.unregisterExtension(extension.key);};
  }, [editor, root]);
  return null;
}
