import { useCallback, useEffect, type ComponentProps } from "react";
import { SuggestionMenu } from "@blocknote/core/extensions";
import { SuggestionMenuController, useBlockNoteEditor, useExtension, useExtensionState, type DefaultReactSuggestionItem } from "@blocknote/react";
import { SuggestionMenuTabConfirmation } from "./SuggestionMenuTabConfirmation";
import { canOpenSuggestionMenu, isSuggestionQueryAllowed } from "./suggestionTriggerPolicy";

type Props = ComponentProps<typeof SuggestionMenuController<(query: string) => Promise<DefaultReactSuggestionItem[]>>>;

/** All typed menus share admission, dismissal and keyboard behavior. Hosts can
 * further restrict admission, but cannot accidentally bypass the base rules. */
export function EditorSuggestionMenu({ shouldOpen, ...props }: Props): React.JSX.Element {
  const editor = useBlockNoteEditor();
  const menu = useExtension(SuggestionMenu);
  const state = useExtensionState(SuggestionMenu);
  const admit = useCallback<NonNullable<Props["shouldOpen"]>>(transaction =>
    editor.isEditable && !editor.prosemirrorView.composing && canOpenSuggestionMenu(transaction, props.triggerCharacter)
      && (shouldOpen?.(transaction) ?? true), [editor, shouldOpen, props.triggerCharacter]);
  useEffect(() => {
    if (state?.show && state.triggerCharacter === props.triggerCharacter
      && !isSuggestionQueryAllowed(props.triggerCharacter, state.query)) menu.closeMenu();
  }, [menu, props.triggerCharacter, state?.show, state?.triggerCharacter, state?.query]);
  return <>
    <SuggestionMenuTabConfirmation triggerCharacter={props.triggerCharacter} />
    <SuggestionMenuController {...props} shouldOpen={admit} />
  </>;
}
