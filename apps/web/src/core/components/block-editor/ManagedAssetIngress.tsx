import {useEffect, useRef} from "react";
import {getBlockInfoAtNearest, getNodeId} from "@blocknote/core";
import type {EditorView} from "@tiptap/pm/view";
import {embeddedImageClipboardFile, isImageClipboardHTML} from "./ImageClipboard";
import {notify} from "../../services/notifications";

interface AssetEditor {
  readonly domElement: HTMLElement | undefined;
  readonly prosemirrorView: EditorView;
  readonly isEditable: boolean;
  getTextCursorPosition(): {readonly block: {readonly id: string}};
  getBlock(id: string): {readonly id: string} | undefined;
  onMount(callback: () => void): () => void;
}

export interface AssetInsertion {readonly blockId: string; readonly placement: "before" | "after" | "replaceEmpty"}

/** Files enter the host's durable importer before becoming editor content.
 * Rich clipboard blocks and internal block moves keep BlockNote's handlers. */
export function ManagedAssetIngress({editor, onFiles}: {
  readonly editor: AssetEditor;
  readonly onFiles: (files: readonly File[], insertion: AssetInsertion) => void;
}): null {
  const current = useRef(onFiles); current.current = onFiles;
  useEffect(() => {
    let detach: (() => void) | undefined;
    const attach = () => {
      detach?.();
      const root = editor.domElement;
      if (!root) return;
      const receive = (event: ClipboardEvent | DragEvent) => {
        // Restricted native text blocks own their input. A caption/title is
        // never an alternate file-import target for the enclosing document.
        if (event.target instanceof Element && event.target.closest("[data-restricted-block-text]")) return;
        const data = "clipboardData" in event ? event.clipboardData : event.dataTransfer;
        if (!data || data.getData("blocknote/html")) return;
        const html = data.getData("text/html");
        if (event.type === "paste" && isImageClipboardHTML(html)) {
          // Keep the real paste event in BlockNote's pipeline. Its UniqueID
          // DOM hook must run before parsing copied blocks; consuming the event
          // here skips that hook and exposes duplicate IDs to HBS validation.
          // The shared pasteHandler selects lossless HTML after those hooks.
          return;
        }
        const files = Array.from(data.files ?? []);
        if (!files.length) for (const item of Array.from(data.items ?? [])) {
          if (item.kind === "file") { const file = item.getAsFile(); if (file) files.push(file); }
        }
        if (!files.length && event.type === "paste") {
          try {
            const embedded = embeddedImageClipboardFile(html);
            if (embedded) files.push(embedded);
          } catch (cause) {
            event.preventDefault(); event.stopImmediatePropagation();
            notify.error(cause instanceof Error ? cause.message : String(cause));
            return;
          }
        }
        if (!files.length) return;
        event.preventDefault(); event.stopImmediatePropagation();
        if (!editor.isEditable) return;
        let insertion: AssetInsertion;
        if (event.type === "paste") insertion = {blockId: editor.getTextCursorPosition().block.id, placement: "replaceEmpty"};
        else {
          const drop = event as DragEvent;
          const element = event.target instanceof Element ? event.target.closest<HTMLElement>("[data-id]") : null;
          let id = element && root.contains(element) ? element.dataset.id : undefined;
          if (!id) {
            const hit = editor.prosemirrorView.posAtCoords({left: drop.clientX, top: drop.clientY});
            if (!hit) return;
            const state = editor.prosemirrorView.state;
            id = getNodeId(getBlockInfoAtNearest(state, hit.pos).bnBlock.node, state.doc);
          }
          if (!editor.getBlock(id)) return;
          const target = element ?? Array.from(root.querySelectorAll<HTMLElement>("[data-id]")).find(candidate => candidate.dataset.id === id);
          const rect = target?.getBoundingClientRect();
          insertion = {blockId: id, placement: rect && drop.clientY < (rect.top + rect.bottom) / 2 ? "before" : "after"};
        }
        current.current(files, insertion);
      };
      root.addEventListener("paste", receive, true); root.addEventListener("drop", receive, true);
      detach = () => {root.removeEventListener("paste", receive, true); root.removeEventListener("drop", receive, true);};
    };
    const unmount = editor.onMount(attach); attach();
    return () => {unmount(); detach?.();};
  }, [editor]);
  return null;
}
