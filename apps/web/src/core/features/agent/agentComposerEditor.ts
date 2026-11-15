import { history, invertedEffects } from "@codemirror/commands";
import { EditorState, StateEffect, StateField, Transaction, type ChangeSpec, type Extension } from "@codemirror/state";
import { Decoration, EditorView, WidgetType } from "@codemirror/view";
import type { AgentInputDraft, AgentMentionBinding, AgentReferenceInput } from "@research-assistant/core/agent";
import { composerClipboardSelection, pasteComposerDraft, readComposerClipboard, writeComposerClipboard } from "./agentComposerClipboard";
import {composerNoteId, ComposerNoteReferenceWidget} from "./ComposerNoteReference";

export const composerBindingsEffect = StateEffect.define<readonly AgentMentionBinding[]>();
export const composerBindings = StateField.define<readonly AgentMentionBinding[]>({
  create: () => [],
  update(bindings, transaction) {
    const explicit = transaction.effects.filter(effect => effect.is(composerBindingsEffect)).at(-1);
    if (explicit) return explicit.value;
    if (!transaction.docChanged) return bindings;
    return bindings.flatMap(binding => {
      let touched = false;
      transaction.changes.iterChangedRanges((from, to) => {
        if (from < binding.endUtf16 && to > binding.startUtf16
          || from === to && from > binding.startUtf16 && from < binding.endUtf16) touched = true;
      });
      return touched ? [] : [{...binding,
        startUtf16: transaction.changes.mapPos(binding.startUtf16, 1),
        endUtf16: transaction.changes.mapPos(binding.endUtf16, -1)}];
    });
  },
});

/** CodeMirror's atomic ranges handle cursor motion/deletion. This filter also
 * expands partial selections (including cut/paste and IME replacement), so no
 * editor operation leaves a fragment of a selected resource's display text. */
export const wholeComposerReferences = EditorState.transactionFilter.of(transaction => {
  if (!transaction.docChanged || transaction.effects.some(effect => effect.is(composerBindingsEffect))) return transaction;
  const bindings = transaction.startState.field(composerBindings);
  const changes: {from: number; to: number; insert: string}[] = [];
  let adjusted = false;
  transaction.changes.iterChanges((from, to, _fromB, _toB, insert) => {
    const originalFrom = from, originalTo = to;
    for (const binding of bindings) {
      if (from < binding.endUtf16 && to > binding.startUtf16
        || from === to && from > binding.startUtf16 && from < binding.endUtf16) {
        from = Math.min(from, binding.startUtf16); to = Math.max(to, binding.endUtf16);
      }
    }
    adjusted ||= from !== originalFrom || to !== originalTo;
    const previous = changes.at(-1);
    if (previous && from < previous.to) {
      previous.to = Math.max(previous.to, to); previous.insert += insert.toString();
    } else changes.push({from, to, insert: insert.toString()});
  });
  return adjusted ? {changes, effects: transaction.effects,
    annotations: [Transaction.userEvent.of(transaction.annotation(Transaction.userEvent) ?? "input"),
      Transaction.addToHistory.of(transaction.annotation(Transaction.addToHistory) !== false)],
    scrollIntoView: transaction.scrollIntoView} : transaction;
});

class ReferenceWidget extends WidgetType {
  constructor(readonly input: AgentReferenceInput, readonly open: (input: AgentReferenceInput) => void) { super(); }
  override eq(other: ReferenceWidget): boolean { return this.open === other.open && JSON.stringify(this.input) === JSON.stringify(other.input); }
  toDOM(): HTMLElement {
    const link = document.createElement("a");
    link.className = "agent-composer-reference";
    link.href = `agent-resource:${encodeURIComponent(JSON.stringify(this.input))}`;
    link.setAttribute("contenteditable", "false");
    link.tabIndex = -1;
    const kind = this.input.type === "skill" ? "技能" : this.input.target.kind === "tool" ? "工具" : "文件";
    link.setAttribute("aria-label", `${kind} ${this.input.name}`);
    link.title = this.input.name;
    const icon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    icon.setAttribute("viewBox", "0 0 24 24"); icon.setAttribute("aria-hidden", "true");
    icon.setAttribute("fill", "none"); icon.setAttribute("stroke", "currentColor"); icon.setAttribute("stroke-width", "1.8");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", this.input.type === "skill" ? "M12 3 3 8v9l9 5 9-5V8ZM3 8l9 5 9-5M12 13v9M7.5 5.5l9 5"
      : this.input.target.kind === "tool" ? "M8 3v5M16 3v5M6 8h12v3a6 6 0 0 1-6 6v5M4 8h16"
      : "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8ZM14 2v6h6M8 13h8M8 17h8");
    const label = document.createElement("span");
    label.className = "agent-composer-reference-label";
    label.textContent = this.input.name;
    icon.append(path); link.append(icon, label);
    link.addEventListener("mousedown", event => event.preventDefault());
    link.addEventListener("click", event => { event.preventDefault(); this.open(this.input); });
    return link;
  }
  override ignoreEvent(): boolean { return true; }
}

export function composerReferenceExtensions(open: (input: AgentReferenceInput) => void): Extension {
  const decorations = StateField.define({
    create: state => decorate(state.field(composerBindings)),
    update: (_value, transaction) => decorate(transaction.state.field(composerBindings)),
    provide: field => [EditorView.decorations.from(field), EditorView.atomicRanges.of(view => view.state.field(field))],
  });
  function decorate(bindings: readonly AgentMentionBinding[]) {
    return Decoration.set(bindings.map(binding => {
      const noteId=composerNoteId(binding.input);
      return Decoration.replace({widget: noteId ? new ComposerNoteReferenceWidget(binding.input,noteId,open) : new ReferenceWidget(binding.input,open)})
        .range(binding.startUtf16,binding.endUtf16);
    }), true);
  }
  const clipboard = EditorView.domEventHandlers({
    copy: copyReferences, cut: copyReferences,
    paste(event, view) {
      if (view.state.readOnly || !event.clipboardData) return false;
      const fragment = readComposerClipboard(event.clipboardData);
      if (!fragment) return false;
      const selection = view.state.selection.main;
      const paste = pasteComposerDraft(composerEditorDraft(view.state), fragment, selection.from, selection.to);
      view.dispatch({changes: {from: paste.from, to: paste.to, insert: fragment.text},
        effects: composerBindingsEffect.of(paste.draft.bindings), selection: {anchor: paste.caret},
        scrollIntoView: true, userEvent: "input.paste"});
      event.preventDefault(); return true;
    },
  });
  function copyReferences(event: ClipboardEvent, view: EditorView): boolean {
    const selection = view.state.selection.main;
    if (selection.empty) return false;
    const copied = composerClipboardSelection(composerEditorDraft(view.state), selection.from, selection.to);
    if (!copied.draft.bindings.length) return false;
    // Never delete a cut until the complete clipboard data has been written.
    event.preventDefault();
    if (!event.clipboardData || !writeComposerClipboard(event.clipboardData, copied.draft)) return true;
    if (event.type === "cut" && !view.state.readOnly) view.dispatch({
      changes: {from: copied.from, to: copied.to}, selection: {anchor: copied.from},
      scrollIntoView: true, userEvent: "delete.cut"});
    return true;
  }
  return [composerBindings, wholeComposerReferences, clipboard, decorations, history(),
    invertedEffects.of(transaction => transaction.docChanged || transaction.effects.some(effect => effect.is(composerBindingsEffect))
      ? [composerBindingsEffect.of(transaction.startState.field(composerBindings))] : [])];
}

export function composerEditorDraft(state: EditorState): AgentInputDraft {
  return {text: state.doc.toString(), bindings: state.field(composerBindings)};
}
/** Small replacement keeps the editor's selection and undo history intact when
 * the shared picker inserts a reference or speech input changes the draft. */
export function composerTextChanges(previous: string, next: string): ChangeSpec {
  let from = 0, oldEnd = previous.length, newEnd = next.length;
  while (from < oldEnd && from < newEnd && previous[from] === next[from]) from++;
  while (oldEnd > from && newEnd > from && previous[oldEnd - 1] === next[newEnd - 1]) { oldEnd--; newEnd--; }
  return {from, to: oldEnd, insert: next.slice(from, newEnd)};
}
