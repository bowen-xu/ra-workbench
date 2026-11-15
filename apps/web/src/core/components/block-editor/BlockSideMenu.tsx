import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { BlockAddButton, BlockControls, BlockDragButton } from "./BlockControls";
import { ControlHint } from "../ControlHint";
import { SideMenuExtension, SuggestionMenu } from "@blocknote/core/extensions";
import {
  DragHandleMenu,
  SideMenu,
  SideMenuController,
  useBlockNoteEditor,
  useComponentsContext,
  useExtension,
  useExtensionState,
  type SideMenuProps,
} from "@blocknote/react";
import { startBlockPointerDrag } from "./blockPointerDrag";
import { getEditorOverlayTarget, getEditorScrollContainer, ownsEditorHoverEvent } from "./editorSurface";
import { useBlockDragAutoScroll } from "./useBlockDragAutoScroll";
import { useBlockHighlight } from "./BlockHighlight";
import { blockReviewSnapshot, blockSideMenuReviewAnchor } from "./blockSideMenuAnchor";
import {NoteBlockActions, NoteBlockActionsContext} from "./NoteBlockActions";
import type {baseBlockEditorSchema} from "./baseSchema";
import {isFixedBlock} from "./blockEditingConstraints";

type MenuFreezeReason = "review-hover" | "menu" | "drag";
type SetMenuFreeze = (reason: MenuFreezeReason, active: boolean) => void;
const SideMenuInteraction = createContext<(active: boolean) => void>(() => undefined);

/** Keep upstream block positioning and global observation, but never open its
 * floating controls for another surface's event. Active gestures own their
 * lifetime independently of hover. No host (chat, dialog, etc.) is special. */
export function BlockSideMenuController(): React.JSX.Element {
  const editor = useBlockNoteEditor();
  const show = useExtensionState(SideMenuExtension, { editor, selector: state => state?.show ?? false });
  const [inside, setInside] = useState(false);
  const [interacting, setInteracting] = useState(false);
  useEffect(() => {
    const doc = editor.prosemirrorView.dom.ownerDocument;
    const observe = (event: Event) => {
      const root = editor.domElement;
      setInside(root !== undefined && ownsEditorHoverEvent(root, event));
    };
    const leave = () => setInside(false);
    doc.addEventListener("mousemove", observe, true);
    doc.addEventListener("pointerover", observe, true);
    doc.defaultView?.addEventListener("blur", leave);
    return () => {
      doc.removeEventListener("mousemove", observe, true);
      doc.removeEventListener("pointerover", observe, true);
      doc.defaultView?.removeEventListener("blur", leave);
    };
  }, [editor]);
  return <SideMenuInteraction.Provider value={setInteracting}><SideMenuController sideMenu={BlockSideMenu} floatingUIOptions={{
    useFloatingOptions: { open: show && (inside || interacting), whileElementsMounted: () => () => undefined,
      middleware: [blockSideMenuReviewAnchor] },
    useTransitionStylesProps: { duration: 0, initial: { opacity: 1 } },
    useTransitionStatusProps: { duration: 0 },
  }} /></SideMenuInteraction.Provider>;
}

function BlockDragHandle(props: SideMenuProps & { readonly setMenuFreeze: SetMenuFreeze }): React.JSX.Element | null {
  const Components = useComponentsContext();
  const editor = useBlockNoteEditor();
  const block = useExtensionState(SideMenuExtension, { editor, selector: (state) => state?.block });
  const cancelDrag = useRef<(() => void) | undefined>(undefined);
  const suppressClick = useRef(false);
  const autoScroll = useBlockDragAutoScroll();
  const highlight = useBlockHighlight();
  const actions = useContext(NoteBlockActionsContext);
  const menuOpen = useCallback((open: boolean) => props.setMenuFreeze("menu", open), [props.setMenuFreeze]);
  useEffect(() => () => cancelDrag.current?.(), []);

  if (Components === undefined || block === undefined) return null;
  const Menu = props.dragHandleMenu ?? DragHandleMenu;

  const handle = <BlockDragButton
    className="bn-button note-block-drag-handle" data-block-id={block.id} draggable={false}
    aria-label="拖动块；点击打开块菜单"
    onDragStart={event => event.preventDefault()} />;
  return <span
    className="note-block-drag-control"
    onKeyDownCapture={(event) => {
      if (event.shiftKey && (event.key === " " || event.key === "Enter")) {
        event.preventDefault();
        event.stopPropagation();
        highlight.toggle(block.id);
      }
    }}
    onClickCapture={(event) => {
      if (event.shiftKey) {
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      if (!suppressClick.current) {
        // Retain the hovered block through the trigger's click. Releasing the
        // pending pointer freeze on pointerup hides the trigger before click.
        props.setMenuFreeze("menu", true);
        // Pointerdown preserves the editor selection. Focus the actual trigger
        // before Ariakit snapshots it, or it treats the editor as the disclosure.
        if (event.target instanceof Element) event.target.closest<HTMLElement>(".note-block-drag-handle")?.focus({preventScroll: true});
        return;
      }
      suppressClick.current = false;
      event.preventDefault();
      event.stopPropagation();
    }}
    onClick={() => props.setMenuFreeze("drag", false)}
    onPointerDownCapture={(event) => {
      if (event.button !== 0 || !event.isPrimary || !(event.target instanceof Element)) return;
      const source = event.target.closest<HTMLElement>(".note-block-drag-handle");
      const root = editor.domElement;
      if (source === null || root === undefined) return;
      // A handle must not begin WebKit's native text selection/focus gesture.
      event.preventDefault();
      cancelDrag.current?.();
      if (event.shiftKey) {
        highlight.toggle(block.id);
        editor.focus();
        return;
      }
      suppressClick.current = false;
      props.setMenuFreeze("drag", true);
      const sourceIds = highlight.dragIds(block.id);
      const workspace = root.closest<HTMLElement>(".block-editor");
      cancelDrag.current = startBlockPointerDrag(source, root, event.nativeEvent, {
        blockCount: sourceIds.length,
        sourceIds,
        canDrop: (target) => highlight.canDrop(sourceIds, target),
        move: (target) => {
          if (highlight.move(sourceIds, target)) highlight.select(sourceIds);
        },
        onActivate: () => {
          suppressClick.current = true;
          if (workspace !== null) workspace.dataset.blockDragging = "true";
          highlight.activate(sourceIds);
          const container = getEditorScrollContainer(root);
          if (container !== null) autoScroll.start(container);
        },
        onFinish: ({click}) => {
          if (workspace !== null) delete workspace.dataset.blockDragging;
          highlight.finish();
          autoScroll.stop();
          if (!click) props.setMenuFreeze("drag", false);
        },
      });
    }}
  >{actions ? <NoteBlockActions editor={editor as unknown as typeof baseBlockEditorSchema.BlockNoteEditor}
    blockId={block.id} {...(actions.sourcePath ? {sourcePath: actions.sourcePath} : {})} trigger={handle} setMenuOpen={menuOpen} /> : <Components.Generic.Menu.Root
    onOpenChange={(isOpen: boolean) => props.setMenuFreeze("menu", isOpen)}
    position="left"
  >
    <Components.Generic.Menu.Trigger>
      {handle}
    </Components.Generic.Menu.Trigger>
    <Menu />
  </Components.Generic.Menu.Root>}</span>;
}

export function BlockSideMenu(props: SideMenuProps): React.JSX.Element | null {
  const Components = useComponentsContext();
  const editor = useBlockNoteEditor();
  const sideMenu = useExtension(SideMenuExtension);
  const suggestionMenu = useExtension(SuggestionMenu);
  const block = useExtensionState(SideMenuExtension, { editor, selector: (state) => state?.block });
  const freezes = useRef(new Set<MenuFreezeReason>());
  const onInteractionChange = useContext(SideMenuInteraction);
  const setMenuFreeze = useCallback<SetMenuFreeze>((reason, active) => {
    const wasFrozen = freezes.current.size > 0;
    if (active) freezes.current.add(reason); else freezes.current.delete(reason);
    onInteractionChange(freezes.current.has("menu") || freezes.current.has("drag"));
    const isFrozen = freezes.current.size > 0;
    if (isFrozen !== wasFrozen) {
      if (isFrozen) sideMenu.freezeMenu(); else sideMenu.unfreezeMenu();
    }
  }, [sideMenu, onInteractionChange]);
  useEffect(() => () => onInteractionChange(false), [onInteractionChange]);
  useEffect(() => {
    const root = editor.domElement;
    const candidate = Array.from(root?.querySelectorAll<HTMLElement>(".bn-block-outer[data-id]") ?? [])
      .find(element => element.dataset.id === block?.id);
    const snapshot = candidate ? blockReviewSnapshot(candidate) : undefined;
    if (!root || !candidate || !snapshot) return;
    // Relocated controls sit beside the readonly snapshot. Keep the upstream
    // menu attached while crossing the displayed old/new block, then release
    // only this hover lock. An open menu or active drag retains its own lock.
    setMenuFreeze("review-hover", true);
    const pointer = (event: PointerEvent) => {
      if (!ownsEditorHoverEvent(root, event)) { setMenuFreeze("review-hover", false); return; }
      const old = snapshot.getBoundingClientRect(), current = candidate.getBoundingClientRect();
      const menu = getEditorOverlayTarget(root)?.querySelector(".bn-side-menu")?.getBoundingClientRect();
      const left = Math.min(old.left, current.left, menu?.left ?? current.left);
      const right = Math.max(old.right, current.right);
      const top = Math.min(old.top, current.top), bottom = Math.max(old.bottom, current.bottom, menu?.bottom ?? current.bottom);
      setMenuFreeze("review-hover", snapshot.isConnected && event.clientX >= left && event.clientX <= right
        && event.clientY >= top && event.clientY <= bottom);
    };
    root.ownerDocument.addEventListener("pointermove", pointer, true);
    return () => {
      root.ownerDocument.removeEventListener("pointermove", pointer, true);
      setMenuFreeze("review-hover", false);
    };
  }, [editor, block?.id, setMenuFreeze]);
  const addBlock = useCallback(() => {
    if (block === undefined) return;
    const isEmpty = Array.isArray(block.content) && block.content.length === 0;
    if (isEmpty) editor.setTextCursorPosition(block);
    else {
      const inserted = editor.insertBlocks([{ type: "paragraph" }], block, "after")[0];
      if (inserted !== undefined) editor.setTextCursorPosition(inserted);
    }
    suggestionMenu.openSuggestionMenu("/");
  }, [block, editor, suggestionMenu]);

  if (block && isFixedBlock(block)) return null;
  if (Components === undefined) return <SideMenu {...props} />;
  return <SideMenu {...props}><BlockControls>
    <ControlHint content="添加块"><BlockAddButton className="bn-button note-block-add-button" aria-label="添加块" onClick={addBlock} /></ControlHint>
    <BlockDragHandle {...props} setMenuFreeze={setMenuFreeze} />
  </BlockControls></SideMenu>;
}
