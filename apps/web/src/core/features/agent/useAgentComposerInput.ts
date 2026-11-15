import { createElement, Fragment, useEffect, useReducer, useRef, type RefObject } from "react";
import { editAgentDraft, insertAgentReference, trimmedAgentDraftInput, serializeAgentSourceReferences, withoutAutomaticTextScreenshot, type AgentInputDraft, type AgentReferenceInput, type AgentResourceScope } from "@research-assistant/core/agent";
import type { AgentResourceGateways } from "./agentResourceGateways";
import type { AgentComposerInputHandle } from "./AgentComposerTextInput";
import { useAgentReferencePicker } from "./useAgentReferencePicker";
import { useAgentCommandPicker } from "./useAgentCommandPicker";
import type { AgentCommandGateway } from "./agentCommandGateway";
import type { ConversationInputDraft } from "./queueComposerDraft";
import type { AgentCommandExecution } from "./useAgentCommandExecution";
import type { AgentCommandIntent } from "./useAgentCommandPicker";
import { ComposerDraftHistory } from "./composerDraftHistory";
import { retainHistoryEvidence, releaseHistoryEvidence } from "./evidenceImageGateway";
export interface AgentComposerDraftAccess {
  readonly subscribe?: (listener: () => void) => () => void;
  readonly draft: () => ConversationInputDraft;
  readonly set: (draft: AgentInputDraft) => void;
  readonly error?: () => string;
}
/** All selected resources retain their position and precise identity in text. */
export function useAgentComposerInput({text, setText, input, access, scope, gateways, commandGateway, commandAfterMessageId, commandExecution, onCommand, onCommandFailure, plainText=false}: {
  readonly onCommandFailure?: () => Promise<void>;
  readonly commandExecution?: AgentCommandExecution;
  readonly onCommand?: (text: string, intent: AgentCommandIntent) => void;
  readonly plainText?: boolean;
  readonly text: string; readonly setText: (text: string) => void;
  readonly input: RefObject<AgentComposerInputHandle | null>; readonly access?: AgentComposerDraftAccess;
  readonly scope?: AgentResourceScope; readonly gateways?: AgentResourceGateways;
  readonly commandGateway?: AgentCommandGateway;
  readonly commandAfterMessageId?: string | undefined;
}) {
  const local = useRef<AgentInputDraft>({text, bindings: []});
  const [, refreshDraft] = useReducer((revision: number) => revision + 1, 0);
  const source = access?.draft() ?? local.current;
  const evidence = source.evidence?.map(withoutAutomaticTextScreenshot);
  local.current = editAgentDraft({text: source.text, bindings: source.bindings,
    ...(source.unboundReferences ? {unboundReferences: source.unboundReferences} : {}), ...(evidence ? {evidence} : {})}, text);
  useEffect(() => {
    if (evidence?.some((item, index) => item !== source.evidence?.[index])) access?.set({...source, evidence});
  }, [source, evidence, access]);
  const editingInputId = access?.draft().queuePlacement?.input.inputId;
  const history = useRef(new ComposerDraftHistory(local.current)), historyOwner = useRef({});
  const boundary = JSON.stringify([scope?.kind, scope?.resourceId, scope?.conversationId, editingInputId, commandAfterMessageId]);
  const previousBoundary = useRef(boundary);
  if (previousBoundary.current !== boundary) {history.current.reset(local.current); previousBoundary.current = boundary;}
  else history.current.observe(local.current);
  const retainHistory = () => retainHistoryEvidence(historyOwner.current, history.current.retainedDrafts().flatMap(draft => draft.evidence ?? []));
  useEffect(() => {retainHistory();});
  useEffect(() => {const owner = historyOwner.current; return () => releaseHistoryEvidence(owner);}, []);
  useEffect(() => {
    if (!editingInputId) return;
    input.current?.focus();
    const end = access?.draft().text.length ?? 0;
    input.current?.setSelectionRange(end, end);
  }, [editingInputId, input, access]);
  function changeDraft(next: AgentInputDraft, restoring = false) {
    if (plainText) next={text:next.text,bindings:[]};
    commands.edited(next.text);
    // CodeMirror owns visible spans; legacy resource items without spans remain
    // part of the complete input until an explicit replacement/acknowledgment.
    if (!next.unboundReferences && local.current.unboundReferences) next = {...next, unboundReferences: local.current.unboundReferences};
    if (!next.evidence && local.current.evidence) next = {...next, evidence: local.current.evidence};
    if (!restoring) history.current.record(next);
    retainHistory(); // Protect handles before the persistent draft releases them.
    local.current = next; setText(next.text); access?.set(next); refreshDraft();
  }
  function historyKeyDown(event: Parameters<typeof picker.keyDown>[0]): boolean {
    if (event.nativeEvent.isComposing || event.altKey || !(event.metaKey || event.ctrlKey)) return false;
    const key = event.key.toLowerCase();
    if (key !== "z" && !(key === "y" && event.ctrlKey && !event.metaKey)) return false;
    if (event.defaultPrevented) return true;
    event.preventDefault(); event.stopPropagation();
    const next = key === "y" || event.shiftKey ? history.current.redo() : history.current.undo();
    if (next) {changeDraft({...next, evidence: next.evidence ?? [], unboundReferences: next.unboundReferences ?? []}, true); input.current?.focus();}
    return true;
  }
  function change(value: string) { changeDraft(editAgentDraft(local.current, value)); }
  function insert(reference: AgentReferenceInput, start?: number, end?: number) {
    changeDraft(insertAgentReference(local.current, reference, start, end));
  }
  const picker = useAgentReferencePicker({text, input, insert, bindings: () => local.current.bindings, ...(scope ? {scope} : {}), ...(gateways ? {gateways} : {})});
  const commands = useAgentCommandPicker({text, input, change, afterMessageId: commandAfterMessageId,
    ...(onCommandFailure ? {onCommandFailure} : {}),
    ...(commandExecution ? {commandExecution} : {}), ...(onCommand ? {onCommand} : {}),
    ...(commandGateway ? {gateway: commandGateway} : {}), ...(scope?.conversationId ? {conversationId: scope.conversationId} : {})});
  return {...picker,
    update: (value: string, caret: number) => { if (!plainText) { picker.update(value, caret); commands.update(value, caret); } },
    historyKeyDown,
    keyDown: (event: Parameters<typeof picker.keyDown>[0]) => historyKeyDown(event) || !plainText && (commands.keyDown(event) || picker.keyDown(event)),
    close: () => { picker.close(); commands.close(); },
    menu: plainText ? null : createElement(Fragment, null, commands.menu, picker.menu),
    consumeCommand: plainText ? () => false : commands.consumeCommand,
    contextUsage: commands.contextUsage,
    commandActivities: commands.commandActivities,
    commandPending: commands.commandPending,
    executeCommand: commands.executeCommand,
    value: text, draft: local.current, changeDraft,
    warning:access?.error?.() ?? "", change, userInput: () => trimmedAgentDraftInput(local.current), submission: () => serializeAgentSourceReferences(local.current, scope)};
}
