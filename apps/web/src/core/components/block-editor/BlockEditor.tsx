import { useCallback, useId, useLayoutEffect, useMemo, useRef, type ReactNode, type MouseEventHandler } from "react";
import { BlockNoteViewRaw, ComponentsContext, TableHandlesController } from "@blocknote/react";
import {NoteTableHandle} from "./TableHandleMenu";
import { LinkToolbarController } from "./LinkToolbarController";
import { BrowserLinkToolbar } from "./BrowserLinkToolbar";
import { blockEditorComponents } from "./blockEditorComponents";
import { BlockColorHistory, BlockEditorFormattingToolbar, BlockEditorFormattingToolbarController } from "./BlockEditorFormattingToolbar";
import "@blocknote/ariakit/style.css";
import "@blocknote/core/fonts/inter.css";
import { BlockHighlight } from "./BlockHighlight";
import { BlockSideMenuController } from "./BlockSideMenu";
import { CompositionGate } from "./compositionGate";
import { canMoveBlocks, indentSelectedBlocks, moveBlocks, type BlockMoveEditor, type BlockMovePolicy } from "./moveBlocks";
import type { BlockTreeNode } from "./blockSelectionRoots";
import { EditorReadOnlyContext } from "./EditorReadOnly";
import { FormulaSelectionShortcut } from "./MarkSelectionAsFormula";
import "./BlockEditor.css";
import "../ReorderMotion.css";
import { EditorSuggestionMenu } from "./EditorSuggestionMenu";
import { SlashSuggestionMenu } from "./SynchronizedSuggestionMenu";
import { useOverlayTarget } from "../overlay/OverlayPortal";
import type { Extension, ExtensionFactoryInstance } from "@blocknote/core";
import type { EditorView } from "@tiptap/pm/view";
import { blockSelectionExtension } from "./blockSelectionExtension";
import { blockDeletionMotion } from "./blockDeletionMotion";

/** Structural projection of a real BlockNote instance. Retains the host's block
 * type without the pinned release's incompatible optional-heading schema bound.
 * Supply useCreateBlockNote/BlockNoteEditor.create output, not a stand-in engine.
 */
export interface BlockEditorInstance<Block extends BlockTreeNode> extends BlockMoveEditor<Block> {
  readonly domElement: HTMLElement | undefined;
  readonly prosemirrorView: EditorView;
  isEditable: boolean;
  onChange(callback: () => void): () => void;
  onSelectionChange(callback: () => void): () => void;
  registerExtension(extension: Extension | ExtensionFactoryInstance): void;
  unregisterExtension(extension: string): void;
}

export interface BlockEditorProps<Block extends BlockTreeNode> {
  /** Host-created session. Keep stable for save echoes; replace on document switch. */
  readonly editor: BlockEditorInstance<Block>;
  readonly onChange: () => void;
  readonly onCompositionChange?: (composing: boolean) => void;
  readonly readOnly?: boolean;
  readonly movePolicy?: BlockMovePolicy;
  /** Set false when children supply an EditorSuggestionMenu. */
  readonly defaultSlashMenu?: boolean;
  /** Set false when children supply a host-specific FormattingToolbarController. */
  readonly defaultFormattingToolbar?: boolean;
  readonly defaultLinkToolbar?: boolean;
  readonly children?: ReactNode;
  readonly beforeEditor?: ReactNode;
  readonly className?: string;
  readonly ariaLabel?: string;
  readonly onClickCapture?: MouseEventHandler<HTMLDivElement>;
  readonly onAuxClickCapture?: MouseEventHandler<HTMLDivElement>;
  readonly components?: typeof blockEditorComponents;
}

// The pinned view widens schema generics with exactOptionalPropertyTypes.
// Confine the compatibility assertion to this view edge, not domain contracts.
const TypedBlockNoteView = BlockNoteViewRaw as unknown as <Block extends BlockTreeNode>(props: {
  readonly editor: BlockEditorInstance<Block>;
  readonly className: string;
  readonly editable: boolean;
  readonly onChange: () => void;
  readonly slashMenu: boolean;
  readonly formattingToolbar: boolean;
  readonly linkToolbar: boolean;
  readonly sideMenu: false;
  readonly tableHandles: false;
  readonly children: ReactNode;
  readonly portalElements: {readonly default: HTMLElement};
}) => React.JSX.Element;

/** Editing only: no persistence, business identity, navigation or resource I/O.
 * The host encodes editor.document in onChange using its own durable contract.
 */
export function BlockEditor<Block extends BlockTreeNode>({
  editor, onChange, readOnly = false, movePolicy, defaultSlashMenu = true, defaultFormattingToolbar = true, defaultLinkToolbar = true,
  children, beforeEditor, className, ariaLabel = "块编辑器", onCompositionChange, onClickCapture, onAuxClickCapture, components = blockEditorComponents,
}: BlockEditorProps<Block>): React.JSX.Element {
  const instanceId = useId();
  const surface = useRef<HTMLDivElement>(null);
  const overlayTarget = useOverlayTarget("surface", surface);
  useLayoutEffect(() => {
    const extension = blockDeletionMotion();
    editor.registerExtension(extension);
    return () => editor.unregisterExtension("block-deletion-motion");
  }, [editor]);
  useLayoutEffect(() => {
    const dom = editor.prosemirrorView.dom;
    if (readOnly) return;
    dom.setAttribute("data-tab-action", "both");
    // Runs after ProseMirror's keymaps. An unavailable indent must not turn
    // into browser focus traversal, including when a toolbar is visible.
    const fallback = (event: KeyboardEvent) => {
      if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {
        event.preventDefault(); event.stopPropagation();
      }
    };
    dom.addEventListener("keydown", fallback);
    return () => { dom.removeAttribute("data-tab-action"); dom.removeEventListener("keydown", fallback); };
  }, [editor, readOnly]);
  useLayoutEffect(() => {
    const extension = blockSelectionExtension((ids, direction) => !movement.current.readOnly
      && indentSelectedBlocks(editor, ids, direction, movement.current.movePolicy));
    editor.registerExtension(extension);
    return () => editor.unregisterExtension(extension.key);
  }, [editor]);
  useLayoutEffect(() => {
    overlayTarget.className = className ?? "";
    overlayTarget.dataset.editorOverlayOwner = instanceId;
  }, [className, instanceId, overlayTarget]);
  // The pinned React view remounts on editable prop changes. Use the public
  // setter for later fences, retaining this session's view, focus and plugins.
  const mountEditable = useMemo(() => !readOnly, [editor]);
  useLayoutEffect(() => {
    if (editor.isEditable !== !readOnly) editor.isEditable = !readOnly;
    // TipTap removes tabindex when updating editable; BlockNote normally
    // reinstalls it by remounting. Keep its keyboard entry point in place.
    if (editor.domElement) editor.domElement.tabIndex = 0;
  }, [editor, readOnly]);
  const gate = useRef(new CompositionGate());
  const compositionChange = useRef(onCompositionChange);
  compositionChange.current = onCompositionChange;
  useLayoutEffect(() => {
    const host = surface.current;
    // React delegates composition events in the bubble phase, including its
    // synthetic capture callback. Gate before the engine prepares IME input.
    const start = () => { gate.current.start(); compositionChange.current?.(true); };
    host?.addEventListener("compositionstart", start, true);
    return () => host?.removeEventListener("compositionstart", start, true);
  }, []);
  const movement = useRef({ movePolicy, readOnly });
  useLayoutEffect(() => { movement.current = { movePolicy, readOnly }; }, [movePolicy, readOnly]);
  const canDrop = useCallback((ids: readonly string[], target: Parameters<typeof canMoveBlocks>[2]) =>
    canMoveBlocks(editor.document, ids, target, movement.current.movePolicy), [editor]);
  const move = useCallback((ids: readonly string[], target: Parameters<typeof canMoveBlocks>[2]) =>
    !movement.current.readOnly && moveBlocks(editor, ids, target, movement.current.movePolicy), [editor]);

  return <EditorReadOnlyContext.Provider value={readOnly}><div ref={surface} className={`block-editor${className ? ` ${className}` : ""}`} data-editor-instance={instanceId}
    role="group" aria-label={ariaLabel}
    onClickCapture={onClickCapture} onAuxClickCapture={onAuxClickCapture}
    onCompositionEnd={() => {
      gate.current.end(() => { if (!readOnly) onChange(); });
      onCompositionChange?.(false);
    }}
  >
    {beforeEditor}
    <BlockHighlight instanceId={instanceId} editor={editor} readOnly={readOnly} canDrop={canDrop} move={move}>
      <ComponentsContext.Provider value={components}>
        <BlockColorHistory>
          <TypedBlockNoteView className="bn-ariakit" editor={editor} editable={mountEditable}
            portalElements={{default: overlayTarget}}
            onChange={() => gate.current.change(() => { if (!readOnly) onChange(); })}
            sideMenu={false} tableHandles={false} formattingToolbar={false} slashMenu={false}
            linkToolbar={false}
          >
            {children}
            {!readOnly && <TableHandlesController tableHandle={NoteTableHandle}/>}
            {defaultLinkToolbar && <LinkToolbarController linkToolbar={BrowserLinkToolbar} />}
            {!readOnly && defaultSlashMenu && <EditorSuggestionMenu triggerCharacter="/" suggestionMenuComponent={SlashSuggestionMenu} />}
            {!readOnly && <FormulaSelectionShortcut />}
            {!readOnly && defaultFormattingToolbar && <BlockEditorFormattingToolbarController formattingToolbar={BlockEditorFormattingToolbar} />}
            {!readOnly && <BlockSideMenuController />}
          </TypedBlockNoteView>
        </BlockColorHistory>
      </ComponentsContext.Provider>
    </BlockHighlight>
  </div></EditorReadOnlyContext.Provider>;
}
