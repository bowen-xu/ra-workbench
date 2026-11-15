import { useCallback, useMemo } from "react";
import { useBlockNoteEditor } from "@blocknote/react";
import {SuggestionMenu} from "@blocknote/core/extensions";
import { EditorSuggestionMenu } from "./EditorSuggestionMenu";
import { createEquationItems } from "./noteEditingCommands";
import type { baseBlockEditorSchema } from "./baseSchema";

/** Keep one dollar trigger active so its second character changes the option
 * instead of restarting the query at the second dollar sign.
 */
export function EquationSuggestionMenu(): React.JSX.Element {
  // React's untyped context drops the schema extension guaranteed by the surface.
  const editor = useBlockNoteEditor() as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
  const [inlineItem, blockItem] = useMemo(() => createEquationItems(editor), [editor]);
  const shouldOpen = useCallback(() => {
    // Read the live extension store: a rapid second `$` extends the existing
    // query to `$$`, rather than restarting it before React rerenders.
    const current = editor.getExtension(SuggestionMenu)?.store.state;
    return !current?.show || current.triggerCharacter !== "$";
  }, [editor]);
  const getItems = useCallback(async (query: string) =>
    query === "" ? [inlineItem] : query === "$" ? [blockItem] : [], [inlineItem, blockItem]);

  return <EditorSuggestionMenu triggerCharacter="$" getItems={getItems} shouldOpen={shouldOpen}
    floatingUIOptions={{ elementProps: { className: "note-equation-menu-popover" } }} />;
}
