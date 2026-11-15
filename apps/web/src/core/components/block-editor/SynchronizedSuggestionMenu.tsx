import { Fragment, useEffect, useRef } from "react";
import { SuggestionMenu } from "@blocknote/core/extensions";
import {
  useBlockNoteContext, useBlockNoteEditor, useComponentsContext, useDictionary,
  useExtension, useExtensionState, useSuggestionMenuKeyboardHandler,
  type DefaultReactSuggestionItem, type SuggestionMenuProps,
} from "@blocknote/react";
import "./SynchronizedSuggestionMenu.css";

/** Reuse BlockNote's keyboard handler and Ariakit items, with one active index
 * for pointer and keyboard navigation. The controller still owns insertion,
 * query clearing, dismissal and placement.
 */
function SynchronizedSuggestionMenu({ items, loadingState, onItemClick, triggerCharacter }: SuggestionMenuProps<DefaultReactSuggestionItem> & {
  readonly triggerCharacter: string;
}): React.JSX.Element {
  const editor = useBlockNoteEditor();
  const menu = useExtension(SuggestionMenu);
  const state = useExtensionState(SuggestionMenu);
  const query = state?.query;
  const components = useComponentsContext()!;
  const dictionary = useDictionary();
  const context = useBlockNoteContext();
  const root = useRef<HTMLDivElement>(null);
  const { selectedIndex, setSelectedIndex, handler } = useSuggestionMenuKeyboardHandler(items, onItemClick);
  const activeIndex = items.length ? Math.min(selectedIndex, items.length - 1) : undefined;

  useEffect(() => { setSelectedIndex(0); }, [query, setSelectedIndex]);
  useEffect(() => {
    if (!state?.show || state.triggerCharacter !== triggerCharacter) return;
    const activeId = activeIndex === undefined ? undefined : `bn-suggestion-menu-item-${activeIndex}`;
    context?.setContentEditableProps?.(props => ({ ...props,
      "aria-activedescendant": activeId,
    }));
    return () => context?.setContentEditableProps?.(props => props?.["aria-activedescendant"] === activeId
      ? { ...props, "aria-activedescendant": undefined } : props);
  }, [activeIndex, query, state?.show, state?.triggerCharacter, triggerCharacter, context?.setContentEditableProps]);
  useEffect(() => {
    const document = editor.domElement?.ownerDocument;
    if (!document) return;
    const navigate = (event: KeyboardEvent) => {
      const state = menu.store.state;
      const editable = editor.domElement;
      if (!state?.show || state.triggerCharacter !== triggerCharacter
        || !(event.target instanceof Node) || !editable?.contains(event.target)) return;
      if (event.isComposing || event.keyCode === 229) return;
      // Confirmation and ordinary Enter belong to the host's Tab policy.
      // This document capture handler owns only menu navigation.
      if (event.key === "Enter") return;
      // Capture before the controller's root handler; always resolve the live
      // editable, whose mounting container may change during view updates.
      if (handler(event)) event.stopImmediatePropagation();
    };
    document.addEventListener("keydown", navigate, true);
    return () => document.removeEventListener("keydown", navigate, true);
  }, [editor, menu, handler, triggerCharacter]);

  const ui = components.SuggestionMenu;
  return <div ref={root} className="synchronized-suggestion-menu-container" onPointerMoveCapture={event => {
    const option = (event.target as Element).closest('[role="option"]');
    if (!option || !root.current?.contains(option)) return;
    const index = Array.from(root.current.querySelectorAll('[role="option"]')).indexOf(option);
    if (index >= 0 && index < items.length) setSelectedIndex(index);
  }}>
    <ui.Root id="bn-suggestion-menu" className="bn-suggestion-menu synchronized-suggestion-menu">
      {items.map((item, index) => <Fragment key={`bn-suggestion-menu-item-${index}`}>
        {item.group !== items[index - 1]?.group && <ui.Label className="bn-suggestion-menu-label">{item.group}</ui.Label>}
        <ui.Item item={triggerCharacter === "/" ? { ...item, subtext: "" } : item} id={`bn-suggestion-menu-item-${index}`}
          className={`bn-suggestion-menu-item${item.size === "small" ? " bn-suggestion-menu-item-small" : ""}`}
          isSelected={index === activeIndex} onClick={() => onItemClick?.(item)} />
      </Fragment>)}
      {loadingState !== "loaded" && <ui.Loader className="bn-suggestion-menu-loader" />}
      {loadingState !== "loading-initial" && items.length === 0 && <ui.EmptyItem className="bn-suggestion-menu-item">
        {dictionary.suggestion_menu.no_items_title}
      </ui.EmptyItem>}
    </ui.Root>
  </div>;
}

export function SlashSuggestionMenu(props: SuggestionMenuProps<DefaultReactSuggestionItem>): React.JSX.Element {
  return <SynchronizedSuggestionMenu {...props} triggerCharacter="/" />;
}

export function MentionSuggestionMenu(props: SuggestionMenuProps<DefaultReactSuggestionItem>): React.JSX.Element {
  return <SynchronizedSuggestionMenu {...props} triggerCharacter="@" />;
}
