import { OverlayPortal } from "../../../components/overlay/OverlayPortal";
import { useNotificationState } from "../../../services/useNotificationState";
import { useEffect, useState } from "react";
import { decodeAgentEvidenceInput, type AgentEvidenceInput } from "@research-assistant/core/agent";
import { formatSourceLink } from "@research-assistant/core/source";
import type { NoteEditorInstance, NoteEditorBlock } from "./noteEditorSchema";
import { useAgentCanvasMessage } from "../../agent/AgentCanvasShell";
import { useNotesClient } from "../NotesClient";
import { noteEvidenceSelection } from "./noteEvidenceSelection";
import "./NoteEvidenceMenu.css";
import { ActionMenu, ActionMenuItem } from "../../../components/ActionMenu";
import { captureEvidenceImage } from "../../agent/evidenceImageGateway";

export function NoteEvidenceMenu({editor, pageId}: {readonly editor: NoteEditorInstance; readonly pageId: string}) {
  const send = useAgentCanvasMessage(), client = useNotesClient();
  const [target, setTarget] = useState<{x: number; y: number; block: NoteEditorBlock; selected: readonly AgentEvidenceInput[]} | null>(null);
  const [busy, setBusy] = useState(false), [, setError] = useNotificationState("error");
  useEffect(() => {
    const element = editor.domElement;
    if (!element) return;
    const capture = (event: MouseEvent) => {
      const id = (event.target as Element).closest<HTMLElement>("[data-id]")?.dataset.id;
      const block = id ? editor.getBlock(id) : editor.getTextCursorPosition().block;
      if (!block) return;
      event.preventDefault();
      event.stopPropagation();
      const state = editor.prosemirrorView.state;
      const selected = state.selection.empty ? [] : noteEvidenceSelection(state.doc, pageId, state.selection.from, state.selection.to);
      setTarget({x: Math.min(event.clientX, window.innerWidth - 210), y: Math.min(event.clientY, window.innerHeight - 160), block: structuredClone(block), selected}); setError("");
    };
    element.addEventListener("contextmenu", capture);
    const keyboard = (event: KeyboardEvent) => {
      if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
      event.preventDefault(); event.stopPropagation();
      const rect = element.getBoundingClientRect();
      const block = editor.getTextCursorPosition().block, state = editor.prosemirrorView.state;
      setTarget({x:rect.left,y:rect.top,block:structuredClone(block),selected:state.selection.empty ? [] : noteEvidenceSelection(state.doc,pageId,state.selection.from,state.selection.to)}); setError("");
    };
    element.addEventListener("keydown",keyboard);
    return () => {element.removeEventListener("contextmenu", capture);element.removeEventListener("keydown",keyboard);};
  }, [editor, pageId]);
  async function add(targetKind: "selected" | "block") {
    if (!target || busy) return;
    setBusy(true); setError("");
    try {
      const block = target.block;
      const props = block.props as Record<string, unknown>;
      const source = formatSourceLink({path: ["notes", pageId], locator: {kind: "block", blockId: block.id}});
      let evidence: readonly AgentEvidenceInput[];
      if (targetKind === "selected") evidence = target.selected;
      else {
        const rendered = block.content ? await editor.blocksToMarkdownLossy([{...block, children: []}]) : "";
        const text = rendered.trim() ? rendered : JSON.stringify({type:block.type, props:block.props, content:block.content});
        const image = block.type === "managedImage" && typeof props.assetId === "string"
          ? await captureEvidenceImage(Array.from(await client.gateway.readNoteAsset(props.assetId)),String(props.mimeType),String(props.displayName||"笔记图片")) : undefined;
        evidence = [decodeAgentEvidenceInput({type: "annotation", comment: "", source, title: image ? String(props.displayName || "笔记图片") : "笔记块", text,
          ...(image ? {image,displayText:String(props.caption || props.displayName || "笔记图片")} : {})})];
      }
      if (!evidence.length) throw new Error("请先选择笔记内容");
      await send("", [{kind: "noteGroup", resourceId: pageId, parts: [{resource: {kind: "note", resourceId: pageId}, selected: true, access: "readOnly", label: "引用的笔记"}]}], {mode: "draft", evidence});
      setTarget(null);
    } catch (reason) {setError(reason instanceof Error ? reason.message : String(reason));}
    finally {setBusy(false);}
  }
  return target ? <OverlayPortal><div className="note-evidence-menu-anchor" style={{left: target.x, top: target.y}}>
    <ActionMenu label="引用笔记内容" className="note-evidence-menu-root" contentClassName="note-evidence-menu" trigger={<span/>} triggerClassName="note-evidence-menu-trigger" open setOpen={open=>{if(!open&&!busy){setTarget(null);editor.focus();}}} autoFocusOnHide={false} portal>
    {target.selected.length > 0 && <ActionMenuItem hideOnClick={false} disabled={busy} onClick={() => void add("selected")}>批注选中文字</ActionMenuItem>}
    <ActionMenuItem hideOnClick={false} disabled={busy} onClick={() => void add("block")}>{target.block.type === "managedImage" ? "批注这张图片" : "批注这个块"}</ActionMenuItem>

    </ActionMenu>
  </div></OverlayPortal> : null;
}
