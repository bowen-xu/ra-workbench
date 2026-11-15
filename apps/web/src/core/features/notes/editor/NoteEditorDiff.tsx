import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { TextSelection } from "@tiptap/pm/state";
import { EditorDiffControls } from "../../../components/editor-diff/EditorDiffControls";
import { createNoteDiffPlugin, noteDiffRanges, type NoteEditorDiffChunk, type NoteDiffPresentation } from "./noteDiffDecorations";
import type { NoteEditorInstance } from "./noteEditorSchema";
import { NoteDiffAssetWidgets, noteDiffAssetTargets, type NoteDiffAssetTarget } from "./NoteDiffAssetWidgets";
import "./NoteEditorDiff.css";

/** Diff extends the already mounted BlockNote view. The host resolves chunks
 * and owns the current document, review baseline, persistence and undo history.
 */
export function NoteEditorDiff({ editor, chunks, disabled = false, onResolve, controlsTarget, onKeepBoth }: {
  readonly editor: NoteEditorInstance;
  readonly chunks: readonly NoteEditorDiffChunk[];
  readonly disabled?: boolean;
  readonly onResolve: (action: "keep" | "undo", chunkId?: string) => void;
  readonly controlsTarget?: HTMLElement | null;
  readonly onKeepBoth?: (id: string) => void;
}): React.JSX.Element | null {
  const live = useRef({ chunks, disabled, onResolve, onKeepBoth, conflict: !!onKeepBoth });
  live.current = { chunks, disabled, onResolve, onKeepBoth, conflict: !!onKeepBoth };
  const pluginRef = useRef<ReturnType<typeof createNoteDiffPlugin> | undefined>(undefined);
  const navigation = useRef<{ readonly id: string; readonly head: number } | undefined>(undefined);
  const [activeId, setActiveId] = useState<string>();
  const [assetTargets, setAssetTargets] = useState<readonly NoteDiffAssetTarget[]>([]);
  const active = Math.max(0, chunks.findIndex(chunk => chunk.id === activeId));
  useEffect(() => {
    const tiptap = editor._tiptapEditor;
    let disposed = false;
    let frame: number | undefined;
    const widgets = new Map<HTMLElement, readonly NoteDiffAssetTarget[]>();
    const publishAssets = () => { if (!disposed) setAssetTargets([...widgets.values()].flat()); };
    const install = () => {
      if (disposed || !editor.domElement) return;
      const plugin = pluginRef.current ?? createNoteDiffPlugin(editor, live.current, {
        onBoth: id => { if (!live.current.disabled) live.current.onKeepBoth?.(id); },
        onResolve: (action, id) => { if (!live.current.disabled) live.current.onResolve(action, id); },
        onSelection: id => {
          if (!id) return;
          navigation.current = { id, head: editor.prosemirrorState.selection.head };
          setActiveId(id);
        },
        onOldWidgetRendered: (widget, chunk) => {
          if (disposed) return;
          const targets = noteDiffAssetTargets(widget, chunk);
          if (targets.length) { widgets.set(widget, targets); publishAssets(); }
        },
        onOldWidgetDestroyed: widget => {
          if (widgets.delete(widget)) queueMicrotask(publishAssets);
        },
      });
      pluginRef.current = plugin;
      // Pinned BlockNote remounts its ProseMirror view when editable changes.
      // TipTap rebuilds that view's static plugin list, dropping runtime
      // decorations. Reattach this same presentation key after each mount and
      // immediately project the latest chunks; B/D/history remain host-owned.
      if (!plugin.key.getState(editor.prosemirrorState)) tiptap.registerPlugin(plugin.plugin);
      editor.prosemirrorView.dispatch(editor.prosemirrorView.state.tr
        .setMeta(plugin.key, { chunks: live.current.chunks, disabled: live.current.disabled, conflict: live.current.conflict })
        .setMeta("addToHistory", false));
    };
    tiptap.on("mount", install);
    // BlockNote mounts its view from a parent effect. Wait one frame when these
    // controls rendered first; no polling or additional editor instance.
    if (editor.domElement) install();
    else frame = requestAnimationFrame(install);
    return () => {
      disposed = true;
      if (frame !== undefined) cancelAnimationFrame(frame);
      tiptap.off("mount", install);
      const plugin = pluginRef.current;
      if (plugin && !tiptap.isDestroyed) tiptap.unregisterPlugin(plugin.key);
      pluginRef.current = undefined;
    };
  }, [editor]);
  useEffect(() => {
    const plugin = pluginRef.current;
    if (!plugin || !editor.domElement) return;
    const presentation: NoteDiffPresentation = { chunks, disabled, conflict: !!onKeepBoth };
    editor.prosemirrorView.dispatch(editor.prosemirrorView.state.tr.setMeta(plugin.key, presentation).setMeta("addToHistory", false));
  }, [editor, chunks, disabled, onKeepBoth]);

  const move = (direction: "previous" | "next") => {
    if (disabled || !editor.domElement) return;
    const view = editor.prosemirrorView;
    const ranges = noteDiffRanges(view.state, chunks);
    const cursor = view.state.selection.head;
    const navigated = navigation.current?.head === cursor ? ranges.findIndex(range => range.id === navigation.current?.id) : -1;
    const current = navigated >= 0 ? navigated : ranges.findIndex(range => range.from <= cursor && cursor <= range.to);
    const index = direction === "next"
      ? current >= 0 ? current + 1 : ranges.findIndex(range => range.from > cursor)
      : current >= 0 ? current - 1 : ranges.reduce((last, range, index) => range.to < cursor ? index : last, -1);
    const target = ranges[index];
    if (!target) return;
    // Root-block boundaries need the nearest valid text selection. Pure
    // deletions still navigate to their stable insertion boundary.
    const selection = TextSelection.near(view.state.doc.resolve(target.from + (target.from < view.state.doc.content.size ? 1 : 0)));
    navigation.current = { id: target.id, head: selection.head };
    view.dispatch(view.state.tr.setSelection(selection).scrollIntoView());
    setActiveId(target.id);
    view.focus();
  };
  if (!chunks.length) return null;
  const controls = <div className="note-editor-diff-controls">
    <EditorDiffControls count={chunks.length} active={active + 1} disabled={disabled} onMove={move}
      onResolve={(action, scope) => onResolve(action, scope === "all" ? undefined : chunks[active]?.id)} />
  </div>;
  return <>{!onKeepBoth && (controlsTarget ? createPortal(controls, controlsTarget) : controls)}<NoteDiffAssetWidgets targets={assetTargets} /></>;
}
