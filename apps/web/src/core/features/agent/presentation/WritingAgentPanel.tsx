import type { WritingAgentPanelProps } from "./writingAgentPanelTypes";
export type { WritingAgentSubmitOptions } from "./writingAgentPanelTypes";
import {useAgentTranscriptScroll} from "./useAgentTranscriptScroll";
import { Dialog } from "../../../public/overlay";
import {SystemDocumentApprovalPanel} from "../../../public/settings";

import { ConversationHeaderTitle } from "../../../public/agentConversations/ConversationHeaderTitle";
import { ControlHint } from "../../../public/overlay";
import { AgentReferenceCountBadge, useAgentDraftReferenceCount } from "../../../public/agentSubmission/AgentDraftReferenceCount";
import { AgentActivityIcon, agentLauncherDescription, usePublishAgentActivity } from "../../../public/agentPresentation/AgentActivity";
import { AgentComposerInputState } from "../../../public/agentSubmission/AgentComposerInputState";
import { AgentComposerTextInput, type AgentComposerInputHandle, type AgentComposerKeyEvent } from "../../../public/agentSubmission/AgentComposerTextInput";
import { AgentMessageSelectionAction } from "../../../public/agentPresentation/AgentMessageSelectionAction";
import { ArrowUp, GripVertical, MessageCircle, Minus, Network, Pin, Square, Target, X } from "lucide-react";
import { useAgentChatPin } from "../../../public/agentPresentation/AgentChatPin";
import {ReturnToBottomButton} from "./ReturnToBottomButton";
import {useReturnToBottomVisibility} from "./useReturnToBottomVisibility";
import { Fragment, useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore, type CSSProperties, type FormEvent, type KeyboardEvent } from "react";
import { AgentCanvasNavigationFailure } from "../../../public/agentPresentation/AgentCanvasNavigationError";
import type { LlmModelSelection } from "@research-assistant/core/ai";

import { PaperAiReadingSpeechInput } from "../../../public/speech";
import type { SpeechInputState } from "../../../public/speech";
import { WritingAgentContextControl, type WritingAgentContextScope } from "./WritingAgentContextControl";
import { WritingAgentTranscript } from "./WritingAgentTranscript";
export type { WritingAgentMessage } from "./WritingAgentMessage";
import { useAgentComposerInput } from "../../../public/agentSubmission/useAgentComposerInput";
import type { AgentCommandIntent } from "../../../public/agentSubmission/useAgentCommandPicker";
import { compactRunningMessage } from "../../../public/agentSubmission/dispatchQueuedAgentCommand";
import { notify } from "../../../public/feedback";
import type { AgentInputDraft, AgentUserInput } from "@research-assistant/core/agent";
import { WritingAgentComposerAddMenu } from "./WritingAgentComposerAddMenu";
import { WritingAgentModelControl } from "./WritingAgentModelControl";

import "./WritingAgentPanel.css";
import { useWritingAgentDock } from "./useWritingAgentDock";
import { useWritingAgentSize } from "./useWritingAgentSize";
import { useMessageSearchTarget } from "../../../public/agentPresentation/useMessageSearchTarget";
import { useShortcutBindings } from "../../../public/settings";
import { formatShortcut } from "../../../public/settings";

const noDraftSubscription = (): (() => void) => () => undefined;
const emptyComposerDraft: AgentInputDraft = {text:"", bindings:[]};

const CHAT_OVERLAYS = ".writing-agent-trigger, [data-agent-conversation-trigger], .agent-canvas-conversations:not([hidden]), .agent-reference-picker, .agent-command-picker, .writing-agent-composer-add-menu, .agent-model-picker-popover, .agent-context-dialog, .agent-message-selection-action, .agent-evidence-popover, .annotation-comment-popover";

function WritingAgentComposerActiveOptions({ goalMode, includeConceptRetrieval, onClearGoal, onClearConceptRetrieval }: {
  readonly goalMode: boolean;
  readonly includeConceptRetrieval: boolean;
  readonly onClearGoal: () => void;
  readonly onClearConceptRetrieval: () => void;
}): React.JSX.Element | null {
  if (!goalMode && !includeConceptRetrieval) return null;
  return <div aria-label="已启用的 Agent 选项" className="writing-agent-composer-active-options">
    {goalMode && <ControlHint content="清除目标"><button aria-label="清除目标" className="writing-agent-composer-active-option" onClick={onClearGoal}  type="button"><Target aria-hidden="true" size={17} /><X aria-hidden="true" className="writing-agent-composer-active-option-clear" size={15} /></button></ControlHint>}
    {includeConceptRetrieval && <ControlHint content="清除概念网络检索"><button aria-label="关闭概念网络检索" className="writing-agent-composer-active-option" onClick={onClearConceptRetrieval}  type="button"><Network aria-hidden="true" size={17} /><X aria-hidden="true" className="writing-agent-composer-active-option-clear" size={15} /></button></ControlHint>}
  </div>;
}

export function WritingAgentPanel({
  onSubmitCommand,
  activeTaskKind,
  approvalConversationId,
  independentContext = false,
  headerControls,
  showLauncher = true,
  dismissOnOutsideClick = true,
  inputDisabled = false,
  suppressPreparing = false,
  isSubmitting: sending = false,
  queuePreparation,
  hasPendingSubmission = false,
  submittingText = "正在核对请求……",
  turnAttachment, turnFeedback, turnAttachmentPaths, messageExtras, readMessageData, messageActions, historyActions, historyHeader,
  searchTarget, historyReady = true, historyError = false,
  activePath,
  contextUsage,
  modelGateway,
  resourceGateways, commandGateway, commandExecution, onQueueCommand, draftResources, resourceScope,
  disabledReason,
  isRunning = false,
  thinkingStartedAtMs,
  conversationTitle,
  conversationRename,
  messages = [],
  childTasks = [],
  onOpenChildTask,
  onReconcileChildTask,
  onResumeChildTask,
  onCancel,
  onCancelSending,
  cancelSendingPending = false,
  onSteer,
  steeringPending = false,
  onOpenChange,
  onSubmit,
  onModelSelectionChange,
  open: controlledOpen,
  defaultSize,
  draft: controlledDraft,
  onDraftChange,
  submissionControls,
  composerAccessory,
  contextControls,
  onQueue,
  queueDraftEditing = false,
  queueDisabled = false,
}: WritingAgentPanelProps): React.JSX.Element {
  const isSubmitting = sending || !!queuePreparation || cancelSendingPending;
  const panelId = useId();
  const shortcuts = useShortcutBindings();
  const [pinned, setPinned] = useAgentChatPin();
  const [internalOpen, setInternalOpen] = useState(false);
  const open = controlledOpen ?? internalOpen;
  const [hasOpened, setHasOpened] = useState(open);
  useLayoutEffect(() => { if (open) setHasOpened(true); }, [open]);
  const docking = useWritingAgentDock(open);
  const { heightPreset, widthPreset, widthResizing, currentWidth, currentHeight, maximumWidth, maximumHeight,
    minimumWidth, minimumHeight, adjustWidth, adjustHeight,
    onResizePointerDown, onResizePointerMove, onResizePointerUp,
    edgeResizeProps } = useWritingAgentSize(defaultSize, docking.dock !== "right");
  const resizeSides = docking.dock === "right" ? ["left"] as const : ["left", "right"] as const;
  const [internalDraft, setInternalDraft] = useState("");
  const storedDraft = useSyncExternalStore(draftResources?.subscribe ?? noDraftSubscription,
    () => draftResources?.subscribe ? draftResources.draft() : emptyComposerDraft);
  const draft = controlledDraft ?? (draftResources?.subscribe ? storedDraft.text : internalDraft);
  function setDraft(value: string): void {
    if (controlledDraft === undefined) setInternalDraft(value);
    onDraftChange?.(value);
  }
  const [contextScope, setContextScope] = useState<WritingAgentContextScope>(activePath === undefined ? "manuscript" : "current-file");
  const [goalMode, setGoalMode] = useState(false);
  const [modelSelection, setModelSelection] = useState<LlmModelSelection | undefined>(undefined);
  useEffect(() => { onModelSelectionChange?.(modelSelection); }, [modelSelection, onModelSelectionChange]);
  const [includeConceptRetrieval, setIncludeConceptRetrieval] = useState(false);
  const [speechInputState, setSpeechInputState] = useState<SpeechInputState>("idle");
  const contextScopeWasChosenRef = useRef(false);
  const composerRef = useRef<AgentComposerInputHandle>(null);
  const sendFocus = useRef<{scope: string; active: Element | null; waiting: boolean; canceled: boolean} | null>(null);
  const sendScope = JSON.stringify([approvalConversationId ?? resourceScope?.conversationId ?? null,
    resourceScope?.kind ?? null, resourceScope?.resourceId ?? null]);
  const inputResources = useAgentComposerInput({text: draft, setText: setDraft, input: composerRef, commandAfterMessageId: messages.at(-1)?.id, onCommand: submitCommand, ...(commandExecution ? {commandExecution} : {}), ...(draftResources ? {access: draftResources} : {}), ...(resourceScope ? {scope: resourceScope} : {}), ...(resourceGateways ? {gateways: resourceGateways} : {}), ...(commandGateway ? {commandGateway} : {})});
  const referenceCount = useAgentDraftReferenceCount(inputResources.draft);
  const visibleContextUsage = inputResources.contextUsage ?? contextUsage;
  function dispatchInput(callback:((text:string,input?:readonly AgentUserInput[])=>void)|undefined,text:string){
    const submitted=inputResources.submission();if(submitted.userInput.length)callback?.(submitted.text,submitted.userInput);else callback?.(text);
  }

  const {messagesRef, transcriptContentRef, awayFromBottom, returnToBottom} = useAgentTranscriptScroll({
    open, historyReady, historyError, searchTarget, messages, childTasks,
  });
  const returnButton = useReturnToBottomVisibility(messagesRef, transcriptContentRef, open);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const restoreTriggerFocus = useRef(false);
  const composerDisabled = !isRunning && (!historyReady || historyError);
  // Wait for the complete send lifecycle, not a transient read state. Restore
  // focus only if the user has not moved elsewhere or opened another modal.
  useLayoutEffect(() => {
    const intent = sendFocus.current;
    if (!intent) return;
    if (!open || intent.scope !== sendScope) {sendFocus.current = null; return;}
    if (isSubmitting) {intent.waiting = true; return;}
    if (!intent.waiting) return;
    sendFocus.current = null;
    const active = document.activeElement;
    const inputElement = composerRef.current?.element ?? (composerRef.current instanceof HTMLElement ? composerRef.current : null);
    const otherModalOpen = Array.from(document.querySelectorAll('dialog[open], [aria-modal="true"]'))
      .some(modal => !inputElement || !modal.contains(inputElement));
    if (!intent.canceled && !composerDisabled && !inputDisabled && !otherModalOpen
      && (active === document.body || active === intent.active || active === composerRef.current?.element || active === composerRef.current)) composerRef.current?.focus();
  }, [open, sendScope, isSubmitting, composerDisabled, inputDisabled]);
  useEffect(() => {
    const userMoved = (event: Event) => {
      if (event.target instanceof Element && event.target.closest('[data-action="cancel-send"]')) return;
      if (sendFocus.current) sendFocus.current.canceled = true;
    };
    document.addEventListener("pointerdown", userMoved, true);
    document.addEventListener("mousedown", userMoved, true);
    document.addEventListener("touchstart", userMoved, true);
    document.addEventListener("focusin", userMoved, true);
    window.addEventListener("blur", userMoved);
    return () => {
      document.removeEventListener("pointerdown", userMoved, true);
      document.removeEventListener("mousedown", userMoved, true);
      document.removeEventListener("touchstart", userMoved, true);
      document.removeEventListener("focusin", userMoved, true);
      window.removeEventListener("blur", userMoved);
    };
  }, []);
  const preparingText = cancelSendingPending ? "正在停止发送……" : queuePreparation ? queuePreparation.status === "sending" ? "发送中…" : "消息发送状态待确认"
    : isSubmitting ? submittingText : composerDisabled && !suppressPreparing && draft.length === 0 ? "准备中……" : null;
  const hasDraft = draft.trim().length > 0;
  const taskActive = isRunning || inputResources.commandPending;
  const launcherActive = isSubmitting || taskActive;
  usePublishAgentActivity(launcherActive);
  const canSubmit = hasDraft
    && !composerDisabled
    && !isSubmitting
    && disabledReason === undefined
    && onSubmit !== undefined
    && !taskActive
    && speechInputState === "idle";
  const canQueue = !!onQueue && !queueDisabled && !steeringPending && hasDraft
    && !composerDisabled && !isSubmitting
    && disabledReason === undefined && speechInputState === "idle";
  const canSteer = taskActive && !inputResources.commandPending && !!onSteer && !steeringPending && hasDraft
    && !isSubmitting && disabledReason === undefined && speechInputState === "idle";
  const canDirectSteer = !inputResources.commandPending && !!onSteer && !steeringPending && draft.trim().length > 0
    && !composerDisabled && !isSubmitting && disabledReason === undefined && speechInputState === "idle";
  useEffect(() => {
    if (open) composerRef.current?.focus();
    else if (restoreTriggerFocus.current) {
      restoreTriggerFocus.current = false;
      triggerRef.current?.focus();
    }
  }, [open]);

  const searchNavigation = useMessageSearchTarget(searchTarget, open, messagesRef, messages, historyReady, historyError);

  function setOpen(next: boolean): void {
    if (controlledOpen === undefined) setInternalOpen(next);
    onOpenChange?.(next);
  }

  useEffect(() => {
    if (activePath === undefined) {
      setContextScope("manuscript");
      return;
    }
    if (!contextScopeWasChosenRef.current) setContextScope("current-file");
  }, [activePath]);

  function hide(): void {
    restoreTriggerFocus.current = true;
    setOpen(false);
  }

  function submit(event?: FormEvent): void {
    event?.preventDefault();
    if (isSubmitting) return;
    if (inputResources.consumeCommand()) return;
    const value = draft.trim();
    if (queueDraftEditing) {
      if (canQueue) dispatchInput(onQueue, value);
      return;
    }
    if (taskActive) {
      // When a queue exists, a temporary queue lock must not turn Enter into
      // a different delivery action. Direct steering remains explicit.
      if (canQueue) dispatchInput(onQueue,value);
      else if (!onQueue && canSteer) dispatchInput(onSteer,value);
      return;
    }
    if (!canSubmit) return;
    sendFocus.current = {scope: sendScope, active: document.activeElement, waiting: false, canceled: false};
    const submitted = inputResources.submission();
    const accepted = onSubmit?.(submitted.text, {
      ...(submitted.userInput.length ? {userInput:submitted.userInput} : {}),
      contextScope,
      ...(goalMode ? { goalMode: true } : {}),
      ...(includeConceptRetrieval ? { includeConceptRetrieval: true } : {}),
      ...(modelSelection === undefined ? {} : { modelSelection }),
    });
    if (accepted === false) sendFocus.current = null;
    // Managed drafts are consumed by their owner after the original receipt.
    // Merely dispatching a send must not erase pending or rejected input.
    if (controlledDraft === undefined && !draftResources && accepted !== false) setDraft("");
  }

  function submitCommand(text: string, intent: AgentCommandIntent): void {
    if (composerDisabled || isSubmitting || disabledReason || steeringPending || speechInputState !== "idle") return;
    if (taskActive && intent === "steer") { notify.error(compactRunningMessage); return; }
    if (taskActive || queueDraftEditing) {
      if (canQueue) {
        if (onQueueCommand) onQueueCommand(text, draft);
        else onQueue?.(text);
      }
      return;
    }
    if (onSubmitCommand) onSubmitCommand(text, { contextScope,
      ...(modelSelection === undefined ? {} : { modelSelection }),
    });
    else void inputResources.executeCommand(draft);
  }

  function onComposerKeyDown(event: AgentComposerKeyEvent): void {
    if (composerDisabled || inputDisabled || isSubmitting) return;
    if (event.nativeEvent.isComposing || inputResources.keyDown(event)) return;
    if (event.key === "Enter" && !event.shiftKey && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      if (canDirectSteer) dispatchInput(onSteer,draft.trim());
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  }

  function onPanelKeyDown(event: KeyboardEvent<HTMLElement>): void {
    // Child controls handle explicit Tab actions before this bubbling guard.
    if (event.key === "Tab" && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      return;
    }
    if (event.key !== "Escape" || !docking.dragging || event.nativeEvent.isComposing || event.defaultPrevented
      || document.querySelector('dialog[open], [aria-modal="true"]')) return;
    // Command menus own Escape; the host must not cancel a turn or hide itself
    // before Ariakit handles menu dismissal.
    if (event.target instanceof HTMLElement && event.target.closest('[role="menu"]')) return;
    // Escape only cancels the layout gesture; it never hides the chat panel.
    if (event.target instanceof HTMLSelectElement) return;
    event.preventDefault();
    event.stopPropagation();
    docking.cancelDrag();
  }

  const launcher = !open && showLauncher ? <ControlHint content={launcherActive ? "正在处理消息，打开 AI 写作助手" : "打开 AI 写作助手"}><button
      aria-controls={panelId}
      aria-expanded="false"
      aria-label="打开 AI 写作助手"
      aria-busy={launcherActive}
      aria-description={agentLauncherDescription(launcherActive, referenceCount)}
      className="writing-agent-trigger"
      onClick={() => setOpen(true)}
      ref={triggerRef}

      type="button"
    ><AgentActivityIcon active={launcherActive}/><AgentReferenceCountBadge count={referenceCount}/></button></ControlHint> : null;

  // Hide the visited panel through Dialog instead of remounting the transcript.
  return <>{launcher}{(hasOpened || open) && <Dialog
    open={open}
    // Layout transitions must not delay hiding this retained dialog.
    hidden={!open}
    unmountOnHide={false}
    modal={false}
    portal={false}
    backdrop={false}
    autoFocusOnShow={false}
    autoFocusOnHide={false}
    hideOnEscape={false}
    hideOnInteractOutside={(event) => !pinned && dismissOnOutsideClick && event.type === "click" && !docking.dragging && !widthResizing
      && !("nativeEvent" in event ? event.nativeEvent : event).composedPath().some(target => target instanceof Element && target.matches(CHAT_OVERLAYS))
      && !document.querySelector('dialog[open], [aria-modal="true"]')}
    getPersistentElements={() => document.querySelectorAll(CHAT_OVERLAYS)}
    onClose={() => setOpen(false)}
    render={<section ref={docking.panelRef} />}
    aria-label={independentContext ? "研究助手" : "AI 写作助手"}
    aria-modal="false"
    className="writing-agent-panel"
    id={panelId}
    onKeyDown={onPanelKeyDown}
    {...edgeResizeProps}
    role="dialog"
    data-height-preset={heightPreset}
    data-width-preset={widthPreset}
    data-width-resizing={widthResizing ? "true" : "false"}
    data-dock={docking.dock}
    data-dragging={docking.dragging ? "true" : "false"}
    style={{ "--writing-agent-height": `${currentHeight}px`, "--writing-agent-width": `${currentWidth.value}px`, "--writing-agent-drag-offset": `${docking.dragOffset}px` } as CSSProperties}
  >
    <div
      aria-label={independentContext ? "调整研究助手高度" : "调整 AI 写作助手高度"}
      aria-orientation="horizontal"
      aria-valuemax={maximumHeight}
      aria-valuemin={minimumHeight}
      aria-valuenow={currentHeight}
      className="writing-agent-resize"
      onKeyDown={(event) => {
        if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
        event.preventDefault();
        adjustHeight(event.key === "ArrowUp" ? 1 : -1);
      }}
      onPointerCancel={onResizePointerUp}
      onPointerDown={onResizePointerDown}
      onPointerMove={onResizePointerMove}
      onPointerUp={onResizePointerUp}
      role="separator"
      tabIndex={0}
    ><span /></div>
    <header className="writing-agent-header">
      <div className="writing-agent-header-actions">
        <ControlHint content="隐藏" shortcut={formatShortcut(shortcuts.toggleChat)}><button aria-label={independentContext ? "隐藏对话" : "隐藏 AI 写作助手"} onClick={hide} type="button"><Minus aria-hidden="true" size={18} /></button></ControlHint>
      </div>
      {headerControls}
      <div className="writing-agent-header-title">
        <ConversationHeaderTitle key={conversationRename?.conversationId ?? "pending"} title={conversationTitle ?? ""} {...(conversationRename ? {rename: conversationRename} : {})}/>
      </div>
      <div className="writing-agent-header-trailing">
        <div className="writing-agent-header-actions">
          <ControlHint content={pinned ? "取消置顶，恢复自动收起" : "置顶聊天窗，停止自动收起"}><button className="writing-agent-pin" aria-label={pinned ? "取消置顶聊天窗" : "置顶聊天窗"} aria-pressed={pinned} onClick={() => setPinned(value => !value)} type="button"><Pin aria-hidden="true" fill={pinned ? "currentColor" : "none"} /></button></ControlHint>
        </div>
        {contextControls}
        <button aria-label="拖动切换聊天布局" aria-description="向左拖动居中，向右拖动靠右；也可使用左右方向键" className="writing-agent-dock-handle" type="button" {...docking.handleProps}>
          <GripVertical aria-hidden="true" />
        </button>
      </div>
    </header>
    {resizeSides.map((side) => <ControlHint content={`拖动调整聊天窗宽度（当前：${currentWidth.label}）`} key={side}><div
      aria-label={`调整${independentContext ? "研究助手" : " AI 写作助手"}${side === "left" ? "左" : "右"}边界`}
      aria-orientation="vertical"
      aria-valuemax={maximumWidth}
      aria-valuemin={minimumWidth}
      aria-valuenow={currentWidth.value}
      className={`writing-agent-width-handle ${side}`}
      onKeyDown={(event) => {
        if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
        event.preventDefault();
        adjustWidth(event.key === "ArrowLeft" ? -1 : 1);
      }}
      role="separator"
      tabIndex={0}
    /></ControlHint>)}
    {resizeSides.map((side) => <ControlHint content={`拖动调整聊天窗${side === "left" ? "左" : "右"}上角大小`} key={side}><div
      aria-label={`调整${independentContext ? "研究助手" : " AI 写作助手"}${side === "left" ? "左" : "右"}上角大小`}
      className={`writing-agent-corner-handle ${side}`}
      onKeyDown={(event) => {
        if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
        event.preventDefault();
        if (event.key === "ArrowLeft" || event.key === "ArrowRight") adjustWidth(event.key === "ArrowLeft" ? -1 : 1);
        else {
          adjustHeight(event.key === "ArrowUp" ? 1 : -1);
        }
      }}
      role="separator"
      tabIndex={0}
    /></ControlHint>)}
    <div ref={messagesRef} aria-live="polite" className="writing-agent-messages" data-follow-bottom={awayFromBottom ? "false" : "true"}>
      <div ref={transcriptContentRef} className="writing-agent-transcript">
      {historyHeader}
      <AgentCanvasNavigationFailure />
      {childTasks.length > 0 && <section aria-label="子任务" className="writing-agent-child-tasks">
        <strong>子任务</strong>
        {childTasks.map((task) => {
          const label = task.status === "running" ? "运行中" : task.status === "completed" ? "已完成" : task.status === "canceled" ? "已取消" : task.status === "recovery_required" ? "需要恢复" : "失败";
          const content = <><span>{label}</span><small>{task.modelCalls} 次模型调用 · {task.toolCalls} 次工具调用</small></>;
          const entry = onOpenChildTask
            ? <ControlHint content="打开子任务会话"><button className="writing-agent-child-task" onClick={() => onOpenChildTask(task.childConversationId)} type="button" >{content}</button></ControlHint>
            : <div className="writing-agent-child-task">{content}</div>;
          return task.status === "recovery_required"
            ? <div className="writing-agent-child-task-row" key={task.childConversationId}>{entry}
              {onResumeChildTask ? <ControlHint content="使用冻结提交恢复子任务"><button type="button" onClick={() => onResumeChildTask(task.childConversationId)} >恢复并继续</button></ControlHint> : null}
              {onReconcileChildTask ? <ControlHint content="重新核对已提交的终态"><button type="button" onClick={() => onReconcileChildTask(task.childConversationId)} >重新核对</button></ControlHint> : null}
            </div>
            : <Fragment key={task.childConversationId}>{entry}</Fragment>;
        })}
      </section>}
      {searchNavigation.feedback && <p role="status">{searchNavigation.feedback}</p>}
      {messages.length === 0 && !hasPendingSubmission && !isRunning && historyReady && !historyError
        ? <div className="writing-agent-welcome">
          <MessageCircle aria-hidden="true" size={25} />
          <strong>研究助手为您服务</strong>
        </div>
        : null}
      <WritingAgentTranscript messages={messages} isRunning={isRunning} activeTaskKind={activeTaskKind} thinkingStartedAtMs={thinkingStartedAtMs} resourceGateways={resourceGateways}
        commandActivities={inputResources.commandActivities}
        readMessageData={readMessageData} turnAttachment={turnAttachment} turnFeedback={turnFeedback} turnAttachmentPaths={turnAttachmentPaths} messageExtras={messageExtras} messageActions={messageActions}
        matchedId={searchNavigation.matchedId} searchMessageId={searchTarget?.messageId} searchTarget={searchTarget} />
      {historyActions}
      {typeof submissionControls === "function" ? submissionControls({ contextScope, ...(goalMode ? { goalMode: true } : {}), includeConceptRetrieval, ...(modelSelection ? { modelSelection } : {}) }) : submissionControls}
      </div>
      <ReturnToBottomButton visible={returnButton.visible} running={isRunning} onClick={() => {returnToBottom(); returnButton.hide();}}/>
    </div>
    <AgentMessageSelectionAction root={messagesRef} enabled={open && !composerDisabled && !inputDisabled && !isSubmitting} scope={sendScope} onAdd={(text, comment, {focusComposer}) => {
      inputResources.changeDraft({...inputResources.draft, evidence: [...(inputResources.draft.evidence ?? []),
        {type: "annotation", title: "对话选段", text, comment}]});
      if (focusComposer) composerRef.current?.focus();
    }} />
    <div className="writing-agent-composer-area">
      <div className="writing-agent-composer-accessory">
        {typeof composerAccessory === "function" ? composerAccessory({ contextScope, ...(goalMode ? { goalMode: true } : {}), includeConceptRetrieval, ...(modelSelection ? { modelSelection } : {}) }) : composerAccessory}
      </div>
    {(approvalConversationId ?? resourceScope?.conversationId) && <SystemDocumentApprovalPanel conversationId={(approvalConversationId ?? resourceScope?.conversationId)!}
      {...(resourceGateways?.systemDocuments ? {gateway: resourceGateways.systemDocuments} : {})}/>}
    <form className="writing-agent-composer agent-reference-anchor" onSubmit={submit} onKeyDownCapture={event => {
      if (event.target instanceof HTMLElement && event.target.closest(".annotation-comment-editor")) return;
      if (composerDisabled || inputDisabled || isSubmitting) return;
      inputResources.historyKeyDown(event);
    }}>
      {inputResources.menu}{inputResources.warning && <p role="alert">{inputResources.warning}</p>}
      <AgentComposerInputState preparingText={preparingText} {...(queuePreparation?.status === "recovery" && queuePreparation.onRecover
        ? {action: {label: "处理消息", onClick: queuePreparation.onRecover, disabled: queuePreparation.recoveryDisabled ?? false}} : {})}><AgentComposerTextInput
        aria-label={independentContext ? "给研究助手发送消息" : "给 AI 写作助手发送消息"}
        onChange={inputResources.change}
        onKeyDown={onComposerKeyDown}
        disabled={composerDisabled || inputDisabled || isSubmitting}
        placeholder={preparingText ?? (composerDisabled && !suppressPreparing ? "准备中……" : "有什么可以帮您")}
        key={draftResources?.draft().queuePlacement?.input.inputId ?? "new"} inputRef={composerRef}
        rows={2}
        draft={inputResources.draft} onDraftChange={inputResources.changeDraft} onCaretChange={inputResources.update}
       {...(resourceGateways ? {gateways: resourceGateways} : {})} {...(resourceScope ? {scope: resourceScope} : {})} /></AgentComposerInputState>
      <div className="writing-agent-composer-footer">
        <WritingAgentComposerAddMenu
          onReferenceSelect={inputResources.select}
          {...(resourceScope ? {resourceScope} : {})}
          {...(resourceGateways ? { resourceGateways } : {})}
          {...(activePath === undefined ? {} : { activePath })}
          contextScope={contextScope}
          disabled={composerDisabled || isSubmitting || disabledReason !== undefined}
          goalMode={goalMode}
          includeConceptRetrieval={includeConceptRetrieval}
          onContextScopeChange={(nextScope) => {
            contextScopeWasChosenRef.current = true;
            setContextScope(nextScope);
          }}
          onGoalModeChange={setGoalMode}
          onIncludeConceptRetrievalChange={setIncludeConceptRetrieval}
        />
        <WritingAgentComposerActiveOptions goalMode={goalMode} includeConceptRetrieval={includeConceptRetrieval} onClearConceptRetrieval={() => setIncludeConceptRetrieval(false)} onClearGoal={() => setGoalMode(false)} />
        <WritingAgentContextControl
          {...(visibleContextUsage === undefined ? {} : { usage: visibleContextUsage })}
        />
        <div className="writing-agent-composer-tools">
          <WritingAgentModelControl disabled={isRunning || inputDisabled} {...(independentContext ? {route:"research_chat",label:"选择研究助手模型"} : {})} {...(modelGateway === undefined ? {} : { gateway: modelGateway })} onSelection={setModelSelection} />
          <PaperAiReadingSpeechInput
            disabled={composerDisabled || isSubmitting || disabledReason !== undefined || steeringPending}
            onChange={inputResources.change}
            onStateChange={setSpeechInputState}
            value={inputResources.value}
          />
        </div>
        <div className="writing-agent-composer-primary">
          {onCancelSending && (isSubmitting || cancelSendingPending)
            ? <button aria-label="停止发送" aria-busy={cancelSendingPending || undefined} className="writing-agent-send writing-agent-stop" data-action="cancel-send" disabled={cancelSendingPending} onClick={onCancelSending} type="button"><Square aria-hidden="true" /></button>
            : isRunning && !hasDraft
            ? <button aria-label="停止生成" className="writing-agent-send writing-agent-stop" data-action="stop" disabled={onCancel === undefined} onClick={onCancel} type="button"><Square aria-hidden="true" /></button>
            : <button aria-label="发送消息" aria-busy={isSubmitting || undefined} className="writing-agent-send" data-action="send" disabled={isSubmitting || (queueDraftEditing ? !canQueue : taskActive ? !canQueue && (!!onQueue || !canSteer) : !canSubmit)} type="submit"><ArrowUp aria-hidden="true" /></button>}
        </div>
      </div>
    </form>
    </div>
  </Dialog>}</>;
}
