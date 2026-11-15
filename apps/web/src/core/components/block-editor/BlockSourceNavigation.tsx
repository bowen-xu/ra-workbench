import {useEffect, useRef} from "react";
import {TextSelection} from "@tiptap/pm/state";
import type {EditorView} from "@tiptap/pm/view";
import type {SourceLineRange} from "@research-assistant/core/source";
import {sourceBlockSelection} from "./sourceBlockSelection";
import {notify} from "../../services/notifications";

export interface BlockSourceNavigation {
  readonly blockId: string;
  readonly token: number;
  readonly range?: SourceLineRange;
  readonly onLocated?: (location: {readonly getAnchorRect: () => DOMRect | null}) => void;
  readonly onError?: (reason: unknown) => void;
}
interface NavigationEditor {
  readonly domElement: HTMLElement | undefined;
  readonly prosemirrorView: EditorView;
  getBlock(id: string): {readonly id: string} | undefined;
  setTextCursorPosition(id: string, placement: "start"): void;
  onMount?(callback: () => void): () => void;
}

/** Hosts resolve the document identity; all note surfaces share block/range
 * selection, mounted-view readiness and non-text block scrolling. */
export function BlockSourceNavigationController({editor, navigation}: {
  readonly editor: NavigationEditor;
  readonly navigation?: BlockSourceNavigation | undefined;
}): null {
  const handled = useRef<{editor: NavigationEditor; token: number} | undefined>(undefined);
  useEffect(() => {
    if (!navigation || handled.current?.editor === editor && handled.current.token === navigation.token) return;
    let cancelled = false, frame: number | undefined;
    const locate = () => {
      if (cancelled || !editor.domElement?.isConnected) return;
      handled.current = {editor, token: navigation.token};
      try {
        const block = editor.getBlock(navigation.blockId);
        if (!block) throw new Error("来源笔记块不存在或已被删除");
        const view = editor.prosemirrorView;
        if (navigation.range) {
          const range = sourceBlockSelection(view.state.doc, navigation.blockId, navigation.range);
          view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, range.from, range.to)).scrollIntoView());
        } else if ("content" in block && Array.isArray(block.content)) editor.setTextCursorPosition(block.id, "start");
        const element = [...editor.domElement.querySelectorAll<HTMLElement>("[data-id]")].find(candidate => candidate.dataset.id === block.id);
        if (!element) throw new Error("来源笔记块尚未显示");
        view.focus(); element.scrollIntoView({block: "center"});
        navigation.onLocated?.({getAnchorRect: () => element.isConnected ? element.getBoundingClientRect() : null});
      } catch (reason) {
        if (navigation.onError) navigation.onError(reason);
        else notify.error(reason instanceof Error ? reason.message : "无法定位笔记块");
      }
    };
    const schedule = () => {if (frame !== undefined) cancelAnimationFrame(frame); frame = requestAnimationFrame(locate);};
    const stop = editor.onMount?.(schedule); schedule();
    return () => {cancelled = true; stop?.(); if (frame !== undefined) cancelAnimationFrame(frame);};
  }, [editor, navigation]);
  return null;
}
