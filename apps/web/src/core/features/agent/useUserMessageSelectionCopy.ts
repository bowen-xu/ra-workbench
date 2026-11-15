import { useEffect, type RefObject } from "react";
import type { AgentInputDraft } from "@research-assistant/core/agent";
import { composerClipboardSelection, writeComposerClipboard } from "./agentComposerClipboard";
import {selectionClipboard} from "../../components/rich-text/selectionClipboard";
import {notify} from "../../services/notifications";

/** Translate visible labels to bound UTF-16 offsets, expanding partial tags. */
function modelOffset(root: HTMLElement, node: Node, offset: number, end: boolean, draft: AgentInputDraft): number {
  const prefix = root.ownerDocument.createRange();
  prefix.selectNodeContents(root); prefix.setEnd(node, offset);
  const position = prefix.toString().length;
  let visible = 0;
  for (const segment of root.querySelectorAll<HTMLElement>("[data-agent-copy-start]")) {
    const length = segment.textContent?.length ?? 0;
    const start = Number(segment.dataset.agentCopyStart), finish = Number(segment.dataset.agentCopyEnd);
    if (position <= visible) return start;
    if (position < visible + length) return segment.dataset.agentCopyReference === "true"
      ? end ? finish : start : Math.min(finish, start + position - visible);
    visible += length;
    if (position === visible) return finish;
  }
  return draft.text.length;
}

function selectedDraft(root: HTMLElement, draft: AgentInputDraft, range: Range, complete: boolean): AgentInputDraft {
  const from = !complete && root.contains(range.startContainer)
    ? modelOffset(root, range.startContainer, range.startOffset, false, draft) : 0;
  const to = !complete && root.contains(range.endContainer)
    ? modelOffset(root, range.endContainer, range.endOffset, true, draft) : draft.text.length;
  return composerClipboardSelection(draft, from, to).draft;
}

function selectedBubbleText(bubble: HTMLElement, range: Range): string {
  const selected = bubble.ownerDocument.createRange(); selected.selectNodeContents(bubble);
  if (bubble.contains(range.startContainer)) selected.setStart(range.startContainer, range.startOffset);
  if (bubble.contains(range.endContainer)) selected.setEnd(range.endContainer, range.endOffset);
  const fragment = selected.cloneContents();
  fragment.querySelectorAll(".agent-user-message-disclosure, [data-agent-message-actions]").forEach(element => element.remove());
  // Keep readable paragraph and list boundaries when joining multiple messages.
  fragment.querySelectorAll("p, pre, li, blockquote, h1, h2, h3, br").forEach(element => element.append("\n"));
  return fragment.textContent?.trimEnd() ?? "";
}

interface CopyRegistry { readonly messages: Map<HTMLElement, AgentInputDraft>; readonly dispose: () => void; }
const registries = new WeakMap<Document, CopyRegistry>();

function createRegistry(document: Document): CopyRegistry {
  const messages = new Map<HTMLElement, AgentInputDraft>();
  const copy = (event: ClipboardEvent) => {
    if (event.target instanceof Element && event.target.closest('input, textarea, [contenteditable="true"], .cm-editor')) return;
    const selection = document.getSelection();
    if (event.defaultPrevented || !event.clipboardData || !selection || selection.isCollapsed || selection.rangeCount !== 1) return;
    const range = selection.getRangeAt(0);
    // Let the composer and unrelated document selections retain their own handlers.
    const involved = [...messages].filter(([root]) => {
      const disclosure = root.closest(".writing-agent-message-bubble")?.querySelector(".agent-user-message-disclosure");
      return range.intersectsNode(root) || !!disclosure && range.intersectsNode(disclosure);
    });
    if (!involved.length) return;
    const direct = involved.find(([root]) => root.contains(range.startContainer) && root.contains(range.endContainer));
    let result: AgentInputDraft;
    let richHTML: string | undefined;
    if (direct) {
      result = selectedDraft(direct[0], direct[1], range, false);
      if (!result.bindings.length) return;
    }
    else {
      const transcript = involved[0]![0].closest(".writing-agent-transcript");
      if (!transcript || !transcript.contains(range.startContainer) || !transcript.contains(range.endContainer)) return;
      result = {text: "", bindings: []};
      const html: string[] = [];
      for (const article of transcript.querySelectorAll<HTMLElement>(".writing-agent-message")) {
        const bubble = article.querySelector<HTMLElement>(".writing-agent-message-bubble");
        if (!bubble || !range.intersectsNode(bubble)) continue;
        const entry = involved.find(([root]) => bubble.contains(root));
        const disclosure = bubble.querySelector(".agent-user-message-disclosure");
        const complete = bubble.querySelector('.agent-user-message-text[data-collapsed="true"]') !== null
          && !!disclosure && range.intersectsNode(disclosure);
        const markdown = bubble.querySelector<HTMLElement>(".writing-agent-markdown");
        let semantic: ReturnType<typeof selectionClipboard> | undefined;
        try {semantic = markdown ? selectionClipboard(markdown, range) : undefined;}
        catch {
          event.preventDefault();
          notify.error("AGENT_SELECTION_COPY_FAILED：无法完整复制选区，请重试或缩小选择范围。");
          return;
        }
        const fragment = entry ? selectedDraft(entry[0], entry[1], range, complete)
          : {text: semantic?.text ?? selectedBubbleText(bubble, range), bindings: []};
        if (!fragment.text) continue;
        if (semantic) html.push(semantic.html);
        else {
          const paragraph = document.createElement("p"); paragraph.textContent = fragment.text;
          html.push(paragraph.outerHTML);
        }
        const offset = result.text.length + (result.text ? 2 : 0);
        result = {text: result.text + (result.text ? "\n\n" : "") + fragment.text,
          bindings: [...result.bindings, ...fragment.bindings.map(binding => ({...binding,
            startUtf16: binding.startUtf16 + offset, endUtf16: binding.endUtf16 + offset}))]};
      }
      richHTML = html.join("\n");
    }
    if (!result.text) return;
    event.preventDefault();
    if (result.bindings.length) writeComposerClipboard(event.clipboardData, result, richHTML);
    else {
      event.clipboardData.setData("text/plain", result.text);
      if (richHTML !== undefined) event.clipboardData.setData("text/html", richHTML);
    }
  };
  document.addEventListener("copy", copy);
  return {messages, dispose: () => document.removeEventListener("copy", copy)};
}

/** One document listener can combine bound references across message boundaries. */
export function useUserMessageSelectionCopy(element: RefObject<HTMLElement | null>, draft: AgentInputDraft): void {
  useEffect(() => {
    const root = element.current;
    if (!root) return;
    const document = root.ownerDocument;
    let registry = registries.get(document);
    if (!registry) { registry = createRegistry(document); registries.set(document, registry); }
    registry.messages.set(root, draft);
    return () => {
      registry.messages.delete(root);
      if (!registry.messages.size) { registry.dispose(); registries.delete(document); }
    };
  }, [element, draft]);
}
