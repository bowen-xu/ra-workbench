import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { getSelectedBlockRootIds, type BlockTreeNode } from "./blockSelectionRoots";
import { isBlockMarqueeStart, startBlockMarqueeSelection } from "./blockMarqueeSelection";
import { ownsEditorElement } from "./editorSurface";
import type { BlockPlacement } from "./moveBlocks";
import type { EditorView } from "@tiptap/pm/view";
import { closeHistory } from "@tiptap/pm/history";
import { BlockSetSelection } from "./BlockSetSelection";

interface BlockHighlightActions {
  readonly canDrop: (ids: readonly string[], target: BlockPlacement) => boolean;
  readonly move: (ids: readonly string[], target: BlockPlacement) => boolean;
  readonly activate: (ids: readonly string[]) => void;
  readonly select: (ids: readonly string[]) => void;
  readonly selectedIds: () => readonly string[];
  readonly dragIds: (id: string) => readonly string[];
  readonly toggle: (id: string) => void;
  readonly finish: () => void;
}

const BlockHighlightContext = createContext<BlockHighlightActions | undefined>(undefined);

export function useBlockHighlight(): BlockHighlightActions {
  const actions = useContext(BlockHighlightContext);
  if (actions === undefined) throw new Error("Block highlight requires its editor provider");
  return actions;
}

/** Editor-owned feedback survives floating side-menu unmounts and follows the
 * original block ID through moves and undo/redo. It never enters saved content.
 */
export function BlockHighlight({ instanceId, editor, readOnly, children, canDrop, move }: {
  readonly instanceId: string;
  readonly editor: {
    readonly document: readonly BlockTreeNode[];
    readonly domElement: HTMLElement | undefined;
    readonly prosemirrorView: EditorView;
    onChange(callback: () => void): () => void;
    onSelectionChange(callback: () => void): () => void;
    focus(): void;
  };
  readonly canDrop: BlockHighlightActions["canDrop"];
  readonly move: BlockHighlightActions["move"];
  readonly readOnly: boolean;
  readonly children: ReactNode;
}): React.JSX.Element {
  const [draggedIds, setDraggedIds] = useState<readonly string[]>();
  const [selectedIds, setSelectedIds] = useState<readonly string[]>([]);
  const actions = useMemo<BlockHighlightActions>(() => {
    const selected = () => {
      const selection = editor.prosemirrorView.state.selection;
      return selection instanceof BlockSetSelection ? selection.blockIds : [];
    };
    const select = (ids: readonly string[]) => {
      const view = editor.prosemirrorView;
      if (ids.length === 0 && !(view.state.selection instanceof BlockSetSelection)) return;
      const roots = getSelectedBlockRootIds(editor.document, ids);
      const selection = BlockSetSelection.create(view.state.doc, roots, view.state.selection.anchor);
      if (!selection.eq(view.state.selection)) {
        const transaction = view.state.tr.setSelection(selection);
        // A new block gesture ends the preceding typing operation, without
        // adding a document/history item for selection itself.
        if (roots.length > 0 && !(view.state.selection instanceof BlockSetSelection)) closeHistory(transaction);
        view.dispatch(transaction);
      }
    };
    return {
      canDrop,
      move,
      selectedIds: selected,
      activate: setDraggedIds,
      select,
      dragIds: (id) => selected().includes(id) ? [...selected()] : [id],
      toggle: (id) => {
        const ids = selected();
        select(ids.includes(id) ? ids.filter(selected => selected !== id) : [...ids, id]);
      },
      finish: () => setDraggedIds(undefined),
    };
  }, [canDrop, editor, move]);

  // Presentation follows the engine, including undo, redo and ordinary clicks.
  useEffect(() => {
    const sync = () => {
      const next = actions.selectedIds();
      setSelectedIds(previous => previous.length === next.length && previous.every((id, index) => id === next[index]) ? previous : next);
    };
    sync();
    const stopSelection = editor.onSelectionChange(sync);
    const stopChange = editor.onChange(sync);
    return () => { stopSelection(); stopChange(); };
  }, [actions, editor]);

  useEffect(() => {
    const root = editor.domElement;
    if (root === undefined) return;
    const window = root.ownerDocument.defaultView!;
    const workspace = root.closest<HTMLElement>(".block-editor");
    let cancelMarquee: (() => void) | undefined;
    let escaped = false;
    const clear = () => actions.select([]);
    const pointerDown = (event: PointerEvent) => {
      // A new handle gesture temporarily previews another subtree. If it is
      // cancelled, retain the previously completed move's selection.
      if (event.target instanceof Element && ownsEditorElement(root, event.target)
        && event.target.closest(".note-block-drag-control")) return;
      const previous = actions.selectedIds();
      cancelMarquee?.();
      escaped = false;
      clear();
      if (readOnly || !isBlockMarqueeStart(root, event)) return;
      // Preserve ordinary blank-area clicks and their native editing focus.
      // The marquee takes over only after its movement threshold is reached.
      let active = false;
      cancelMarquee = startBlockMarqueeSelection(root, event, {
        onActivate: () => {
          active = true;
          if (workspace !== null) workspace.dataset.blockSelecting = "true";
        },
        onSelect: actions.select,
        onFinish: (cancelled) => {
          cancelMarquee = undefined;
          if (workspace !== null) delete workspace.dataset.blockSelecting;
          if (cancelled) actions.select(escaped ? [] : previous);
          else if (active && actions.selectedIds().length > 0) editor.focus();
        },
      });
    };
    const keyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && (cancelMarquee
        || event.target instanceof Element && ownsEditorElement(root, event.target))) { escaped = true; clear(); }
    };
    window.addEventListener("pointerdown", pointerDown, true);
    window.addEventListener("keydown", keyDown, true);
    return () => {
      escaped = true;
      cancelMarquee?.();
      window.removeEventListener("pointerdown", pointerDown, true);
      window.removeEventListener("keydown", keyDown, true);
    };
  }, [actions, editor, readOnly]);

  const highlightedIds = draggedIds ?? selectedIds;
  return <BlockHighlightContext.Provider value={actions}>
    {highlightedIds.length > 0 && <style data-note-block-highlight={JSON.stringify(highlightedIds)}>{
      // Keep presentation outside ProseMirror-owned DOM: its mutation observer
      // can redraw blocks when temporary attributes are written onto wrappers.
      `${highlightedIds.map((id) => `.block-editor[data-editor-instance="${CSS.escape(instanceId)}"] .bn-block-outer[data-id="${CSS.escape(id)}"]`).join(",")} { --block-selected-content-overlay: rgb(45 125 75 / 10%); --block-selected-image-surface: var(--surface, #fff); border-radius: 6px; background: #eaf2ed; }`
    }</style>}
    <span className="note-block-selection-status" aria-live="polite" aria-atomic="true">{selectedIds.length > 1 ? `已选中 ${selectedIds.length} 个块，可拖动任一已选块的手柄整体移动，Escape 清除选择` : ""}</span>
    {children}
  </BlockHighlightContext.Provider>;
}
