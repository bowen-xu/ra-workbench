import { Minimize2 } from "lucide-react";
import { useEffect, useRef, useState, type RefObject } from "react";
import { ActionMenuItem, InputActionMenu, useActionMenuStore } from "../../components/ActionMenu";
import { notify } from "../../services/notifications";
import type { AgentComposerInputHandle, AgentComposerKeyEvent } from "./AgentComposerTextInput";
import { agentComposerCommands, composerCommandQuery, readComposerCommand } from "./agentComposerCommands";
import type { AgentCommandGateway } from "./agentCommandGateway";
import { useAgentCommandExecution, type AgentCommandExecution } from "./useAgentCommandExecution";
import "./AgentCommandPicker.css";

/** Resolve commands before message dispatch; the host chooses queue or execution. */
export type AgentCommandIntent = "send" | "queue" | "steer";
export function useAgentCommandPicker({ text, input, change, gateway, conversationId, afterMessageId, commandExecution, onCommand, onCommandFailure }: {
  readonly onCommandFailure?: () => Promise<void>;
  readonly commandExecution?: AgentCommandExecution;
  readonly onCommand?: (text: string, intent: AgentCommandIntent) => void;
  readonly text: string;
  readonly input: RefObject<AgentComposerInputHandle | null>;
  readonly change: (text: string) => void;
  readonly gateway?: AgentCommandGateway;
  readonly conversationId?: string;
  readonly afterMessageId?: string | undefined;
}) {
  const local = useAgentCommandExecution({ text, change, afterMessageId, enabled: !commandExecution, ...(onCommandFailure ? {onFailure:onCommandFailure} : {}), ...(gateway ? { gateway } : {}), ...(conversationId ? { conversationId } : {}) });
  const execution = commandExecution ?? local;
  const [query, setQuery] = useState<{ text: string; value: string } | null>(null);
  const dismissedText = useRef<string | null>(null);
  function close(value = text): void { dismissedText.current = value; setQuery(null); }
  const visible = query?.text === text;
  const store = useActionMenuStore({ open: visible, setOpen: open => { if (!open) close(); },
    placement: "top-start", virtualFocus: true, focusLoop: true });
  const candidates = visible ? agentComposerCommands.filter(command => command.name.startsWith(query.value.toLowerCase())) : [];
  const firstItemId = store.useState(state => state.renderedItems.find(item => !item.disabled)?.id);
  useEffect(() => {
    const element = input.current instanceof HTMLElement ? input.current : input.current?.element;
    store.setAnchorElement(element ?? null);
    store.setDisclosureElement(element ?? null);
    // Use Ariakit's registered identity after mount; a button render ID is not
    // necessarily the composite item ID. Keep actual focus in the editor.
    store.setActiveId(candidates.length ? firstItemId ?? null : null);
  }, [store, input, visible, candidates.length, firstItemId]);

  function update(value: string, caret: number): void {
    if (dismissedText.current === value) return;
    const next = composerCommandQuery(value, caret);
    setQuery(next === null ? null : { text: value, value: next });
  }
  function consumeCommand(value = text, intent: AgentCommandIntent = "send"): boolean {
    const command = readComposerCommand(value);
    if (!command) return false;
    close();
    if (command.name === "compact" && !command.arguments) {
      if (onCommand) onCommand("/compact", intent);
      else void execution.compact(text);
    } else if (command.name === "compact") {
      notify.error("/compact 不接受参数，请只输入 /compact。");
    } else {
      notify.error(command.name ? `未知命令：/${command.name}。` : "请选择一个命令。");
    }
    return true;
  }
  function keyDown(event: AgentComposerKeyEvent): boolean {
    if (event.keyCode === 229) return true;
    if (event.nativeEvent.isComposing) return false;
    if (visible && event.key === "Escape") {
      event.preventDefault(); event.stopPropagation(); store.hide(); return true;
    }
    if (visible && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault(); store.move(event.key === "ArrowDown" ? store.next() : store.previous()); return true;
    }
    if (event.shiftKey || event.altKey || (event.key !== "Enter" && event.key !== "Tab")) return false;
    if (visible && candidates.length && !event.metaKey && !event.ctrlKey) {
      event.preventDefault(); event.stopPropagation();
      consumeCommand(`/${candidates[0]!.name}`, event.key === "Tab" ? "queue" : "send");
      return true;
    }
    if (!readComposerCommand(text)) return false;
    event.preventDefault(); event.stopPropagation(); consumeCommand(text, event.metaKey || event.ctrlKey ? "steer" : event.key === "Tab" ? "queue" : "send"); return true;
  }
  return { update, keyDown, consumeCommand, close,
    contextUsage: execution.usage,
    commandActivities: execution.activities,
    commandPending: execution.pending,
    executeCommand: execution.compact,
    // Programmatic editor synchronization is not a new edit after dismissal.
    edited: (value: string) => { if (value !== dismissedText.current) dismissedText.current = null; },
    menu: <>{visible ? <InputActionMenu store={store} label="Agent 命令" className="agent-command-picker">
      <div className="agent-command-picker-title">命令</div>
      {candidates.map(command => <ActionMenuItem key={command.name} className="agent-command-picker-item"
        onMouseDown={event => event.preventDefault()} onClick={() => consumeCommand(`/${command.name}`)}>
        <Minimize2 aria-hidden="true" /><span><strong>/{command.name}</strong><small>{command.description}</small></span>
      </ActionMenuItem>)}
      {!candidates.length && <p className="agent-command-picker-empty">没有匹配的命令</p>}
    </InputActionMenu> : null}</> };
}
