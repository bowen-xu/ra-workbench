import { Popover } from "../../../components/overlay/OverlayPopup";
import { ToolbarItem, usePopoverStore } from "@ariakit/react";
import type { FormattingToolbarProps } from "@blocknote/react";
import { useCallback, useRef, useState } from "react";
import type { AgentEvidenceInput } from "@research-assistant/core/agent";
import { SelectionAskAiButton, SelectionAskAiEditor } from "../../agent/SelectionAskAi";
import { submitEvidenceAnnotation, type SelectionAnnotationIntent } from "../../agent/submitEvidenceAnnotation";
import { useAgentCanvasMessage } from "../../agent/AgentCanvasShell";
import type { SourceNavigationLocation } from "../../agent/sourceNavigation";
import type { NoteEditorInstance } from "./noteEditorSchema";
import { noteEvidenceSelection } from "./noteEvidenceSelection";
import { notify } from "../../../services/notifications";
import { BlockEditorFormattingToolbar, BlockEditorFormattingToolbarController } from "../../../components/block-editor/BlockEditorFormattingToolbar";

/** The captured selection outlives the toolbar when the local comment gains focus. */
export function NoteSelectionAskAi({ editor, pageId }: {
  readonly editor: NoteEditorInstance;
  readonly pageId: string;
}): React.JSX.Element {
  const send = useAgentCanvasMessage();
  const panel = useRef<HTMLDivElement>(null);
  const [target, setTarget] = useState<{
    readonly evidence: readonly AgentEvidenceInput[];
    readonly location: SourceNavigationLocation;
  } | null>(null);
  const popover = usePopoverStore({ open: target !== null, placement: "bottom-start" });
  const open = useCallback(() => {
    const view = editor.prosemirrorView;
    const { from, to, empty } = view.state.selection;
    const evidence = empty ? [] : noteEvidenceSelection(view.state.doc, pageId, from, to);
    if (!evidence.length) { notify.info("请先选择笔记文字。"); return; }
    setTarget({ evidence, location: { getAnchorRect: () => {
      if (!view.dom.isConnected) return null;
      try {
        const first = view.coordsAtPos(from), last = view.coordsAtPos(to);
        const left = Math.min(first.left, last.left), top = Math.min(first.top, last.top);
        return new DOMRect(left, top, Math.max(first.right, last.right) - left, Math.max(first.bottom, last.bottom) - top);
      } catch { return null; }
    } } });
  }, [editor, pageId]);
  const toolbar = useCallback((props: FormattingToolbarProps) => <BlockEditorFormattingToolbar {...props} leadingControls={
    <ToolbarItem render={<SelectionAskAiButton className="bn-ak-button bn-ak-secondary"
      onMouseDown={event => event.preventDefault()} onClick={open} />} />
  } />, [open]);
  const close = () => {
    const restoreFocus = panel.current?.contains(document.activeElement);
    setTarget(null);
    if (restoreFocus) editor.focus();
  };
  async function submit(comment: string, intent?: SelectionAnnotationIntent): Promise<void> {
    if (!target) return;
    await submitEvidenceAnnotation({
      capture: async () => target.evidence,
      resources: [{ kind: "noteGroup", resourceId: pageId, parts: [{
        resource: { kind: "note", resourceId: pageId }, selected: true, access: "readOnly", label: "引用的笔记",
      }] }],
      comment, ...(intent ? { intent } : {}), submit: send, discard: close,
    });
  }
  return <>
    {target === null && <BlockEditorFormattingToolbarController formattingToolbar={toolbar} />}
    {target && <Popover ref={panel} store={popover} portal fixed modal={false} gutter={8} fitViewport
      getAnchorRect={() => target.location.getAnchorRect()}
      hideOnEscape={false} hideOnInteractOutside={false} autoFocusOnHide={false}
      className="annotation-comment-popover" aria-label="问AI：笔记选区">
      <SelectionAskAiEditor title="笔记选区" onCancel={close} onSubmit={submit} />
    </Popover>}
  </>;
}
