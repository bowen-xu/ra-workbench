import {useEffect, useRef, useState} from "react";
import {createPortal} from "react-dom";
import {BlockMediaControls} from "./BlockMediaControls";
import {useEditorReadOnly} from "./EditorReadOnly";
import type {ImageAlignment} from "./ImageBlockIcons";
import {focusMediaBlock, type MediaBlockEditor} from "./mediaBlockFocus";
import {focusBlockCaption} from "./captionBlock";
import "./TableMediaControls.css";

export interface TableMediaEditor extends MediaBlockEditor {
  readonly domElement: HTMLElement | undefined;
  readonly isEditable: boolean;
  getBlock(id: string): {readonly type: string; readonly props: Readonly<Record<string, unknown>>} | undefined;
  updateBlock(id: string, update: {readonly props: {readonly caption?: string; readonly textAlignment?: ImageAlignment}}): unknown;
  onChange(callback: () => void): () => void;
  onMount(callback: () => void): () => void;
}
interface Target {readonly controls: HTMLElement; readonly id: string; readonly alignment: ImageAlignment}

function TableMedia({editor, target}: {readonly editor: TableMediaEditor; readonly target: Target}): React.JSX.Element {
  const captionButton = useRef<HTMLButtonElement>(null);
  const readOnly = useEditorReadOnly() || !editor.isEditable;
  const change = (props: {readonly caption?: string; readonly textAlignment?: ImageAlignment}) => {
    if (!readOnly && editor.isEditable && editor.getBlock(target.id)?.type === "table") editor.updateBlock(target.id, {props});
  };
  return <BlockMediaControls label="表格" alignment={target.alignment}
      captionButtonRef={captionButton} onCaptionEdit={readOnly ? undefined : () => focusBlockCaption(editor, target.id)}
      onAlignmentChange={readOnly ? undefined : textAlignment => change({textAlignment})}
      onReturnFocus={() => {focusMediaBlock(editor, target.id);}}/>;
}

/** Content portals inherit the surface's read-only fence and editor lifetime. */
export function TableMediaControls({editor}: {readonly editor: TableMediaEditor}): React.JSX.Element {
  const [targets, setTargets] = useState<readonly Target[]>([]);
  useEffect(() => {
    let active = true, queued = false;
    let observer: MutationObserver | undefined;
    const refresh = () => {
      if (!active || queued) return;
      queued = true;
      queueMicrotask(() => {
        queued = false;
        if (!active) return;
        const next = Array.from(editor.domElement?.querySelectorAll<HTMLElement>(".note-table-controls-slot") ?? []).flatMap(controls => {
          const id = controls.closest<HTMLElement>("[data-id]")?.dataset.id;
          const block = id ? editor.getBlock(id) : undefined;
          const alignment = block?.props.textAlignment;
          return block?.type === "table" ? [{controls, id: id!,
            alignment: alignment === "center" || alignment === "right" ? alignment : "left" as ImageAlignment}] : [];
        });
        setTargets(prior => prior.length === next.length && prior.every((target, index) =>
          target.controls === next[index]?.controls && target.id === next[index]?.id
            && target.alignment === next[index]?.alignment) ? prior : next);
      });
    };
    const mount = () => {
      observer?.disconnect();
      if (editor.domElement) {
        observer = new MutationObserver(refresh);
        observer.observe(editor.domElement, {childList: true, subtree: true});
      }
      refresh();
    };
    const unmount = editor.onMount(mount), offChange = editor.onChange(refresh);
    mount();
    return () => {active = false; observer?.disconnect(); unmount(); offChange();};
  }, [editor]);
  return <>{targets.map(target => createPortal(<TableMedia editor={editor} target={target}/>, target.controls, target.id))}</>;
}
