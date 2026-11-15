import {SystemDocumentApprovalPanel} from "../../system-documents/SystemDocumentApprovalPanel";
import { getAppRuntime } from "../../../runtime/appRuntime";
import { freezeNewPreviewOwnerIntent } from "../../../runtime/nativePreviewOwnerIntent";
import {recoverRejectedSteer,pauseRejectedSteerQueue,prioritizeRejectedSteerLease} from "../../agent/rejectedSteerQueue";
import { AgentMessageData } from "../../agent/AgentMessageData";
import { AgentCompactionActivity } from "../../agent/AgentCompactionActivity";
import { readMessageInspection } from "../../agent/messageInspectionGateway";
import { ControlHint } from "../../../components/ControlHint";
import { AgentComposerTextInput, type AgentComposerInputHandle } from "../../agent/AgentComposerTextInput";
import {AgentComposerInputState} from "../../agent/AgentComposerInputState";
import type { AgentResourceGateways } from "../../agent/agentResourceGateways";
import {useNoteSteering} from "./useNoteSteering";
import {AgentSteeringReceipt} from "../../agent/AgentSteeringReceipt";
import {reconcileSteeringTransfer} from "../../agent/useAgentInputQueue";
import { useAgentQueueAutoPrepare } from "../../agent/useAgentQueueAutoPrepare";
import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowUp, MessageCircle, Square, X } from "lucide-react";
import type { LlmModelSelection } from "@research-assistant/core/ai";
import { agentConversationRequiresToolRecovery, agentTurnFailureReason, agentTurnReconnecting, agentTurnUnconfirmedReceipts, agentTurnUnconfirmedChildCreations, executionModeForModel, type AgentItemProjection, type AgentNoteApplication, type AgentSearchHit, type AgentNoteRewriteReview, type AgentNoteQueueAdmission } from "@research-assistant/core/agent";
import { noteDocumentPlainText, type ReadingNoteBlock, type NoteBlockV1 } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";
import { useAgentDraftStore, useConversationDraft } from "../../agent/AgentDraftProvider";
import { BoundConversationList } from "../../agent/BoundConversationList";
import { AgentMarkdown } from "../../agent/AgentMarkdown";
import { AgentMessageCopyButton } from "../../agent/AgentMessageCopyButton";
import { agentFailureHint } from "../../agent/agentFailureHint";
import { AgentModelControl } from "../../agent/AgentModelControl";
import { AgentResourcePickerMenu } from "../../agent/AgentResourcePickerMenu";
import { useAgentComposerInput } from "../../agent/useAgentComposerInput";
import { compactRunningMessage, dispatchQueuedAgentCommand, queuedCommandTransfer } from "../../agent/dispatchQueuedAgentCommand";
import { readComposerCommand } from "../../agent/agentComposerCommands";
import type { AgentCommandIntent } from "../../agent/useAgentCommandPicker";

import { useAgentConversation } from "../../agent/useAgentConversation";
import { useConversationMetadata } from "../../agent/useConversationMetadata";
import { ReadingNoteSubmissionController, readingNoteSubmissionOwnerKey, readingNoteSubmissionOwners, type ReadingNoteSubmissionSnapshot } from "../../agent/readingNoteSubmissionController";
import {NoteFrozenSubmission} from "./NoteFrozenSubmission";
import {NoteQueueReview} from "./NoteQueueReview";
import {useNoteInputQueue} from "./useNoteInputQueue";
import {AgentInputQueue} from "../../agent/AgentInputQueue";
import {queueDraftEdit} from "../../agent/queueComposerDraft";
import { NoteSubmissionController, noteSubmissionOwnerKey, noteSubmissionOwners, type NoteSubmissionSnapshot } from "../../agent/noteSubmissionController";
import { agentNoteSubmissionGateway, agentReadingNoteSubmissionGateway, agentSubtaskGateway, agentTurnGateway } from "../../../services/agentGateway";
import { notify } from "../../../services/notifications";
import { PaperAiReadingSpeechInput } from "../../speech/PaperAiReadingSpeechInput";
import type { SpeechInputState } from "../../speech/speechRecognition";
import type { NoteSaveStatus } from "../editor/noteSaveController";
import { NoteUnstartedSubmissions } from "./NoteUnstartedSubmissions";
import { NoteRewriteReview } from "./NoteRewriteReview";
import { AgentTextChangeCard } from "../../agent/AgentTextChangeCard";
import {AgentTurnChangeSummary} from "../../agent/AgentTurnChangeSummary";
import "./NoteAgentPanel.css";
import { clearAgentSessionSelection, readAgentSessionSelection, writeAgentSessionSelection } from "../../agent/agentSessionSelection";
import { useAgentChildTasks } from "../../agent/useAgentChildTasks";
import { useAgentHostLifecycle } from "../../agent/useAgentMountedGuard";
import { pendingBoundConversationId } from "../../agent/boundConversationCreationPending";
import { useNoteConversationCreation } from "./useNoteConversationCreation";
import { ConversationCreationRecovery } from "../../agent/ConversationCreationRecovery";
import { AgentChildCreationRecovery } from "../../agent/AgentChildCreationRecovery";
import { childCreationFailureStage, isMatchingChildCreationFailure } from "../../agent/childCreationFailure";
import { agentFailureDiagnostics } from "../../agent/agentFailureDiagnostics";

interface Props {
  readonly resourceGateways?: AgentResourceGateways;
  readonly snapshot: NoteDocumentSnapshot; readonly saveStatus: NoteSaveStatus;
  /** Ordinary note questions dispatch after the frozen context is prepared. */
  readonly autoDispatch?: boolean;
  readonly onAccept?: (review: AgentNoteRewriteReview) => Promise<void>;
  readonly onUndo?: (review: AgentNoteRewriteReview, application: AgentNoteApplication) => Promise<void>;
  readonly acceptanceBlocked?: boolean;
}
function blocks(snapshot: NoteDocumentSnapshot): { id:string; label:string }[] {
  const result: {id:string;label:string}[] = [];
  const visit = (items: readonly NoteBlockV1[], depth: number) => {
    for (const block of items) {
      const text = noteDocumentPlainText({schemaVersion:1,pageId:snapshot.page.id,blocks:[{...block,children:[]}]}).trim();
      result.push({id:block.id,label:`${"　".repeat(Math.min(depth,3))}${text.slice(0,48) || block.type}`});
      visit(block.children,depth+1);
    }
  };
  visit(snapshot.document.blocks,0); return result;
}

type Host = ({readonly kind:"page"} & Props) | {readonly kind:"reading";readonly resourceGateways?:AgentResourceGateways;readonly note:ReadingNoteBlock;readonly isSaved:boolean;readonly autoDispatch?:boolean};
function identity(host:Host) {
  return host.kind==="page"
    ? {id:host.snapshot.page.id,title:host.snapshot.page.title,revision:host.snapshot.page.revision,saved:host.saveStatus==="saved",bindingKind:"note" as const,label:"AI 笔记助手"}
    : {id:host.note.id,title:host.note.source.paperTitle,revision:host.note.revision,saved:host.isSaved&&host.note.status==="active",bindingKind:"readingNote" as const,label:"AI 阅读笔记助手"};
}

type NoteTimelineItem = { readonly id: string; readonly item: AgentItemProjection; readonly isRewrite: boolean };
type NoteTimelineEntry = { readonly kind: "message"; readonly item: NoteTimelineItem } | { readonly kind: "activity"; readonly items: readonly NoteTimelineItem[] };

function groupNoteTimeline(items: readonly NoteTimelineItem[]): readonly NoteTimelineEntry[] {
  const entries: NoteTimelineEntry[] = [];
  for (let index = 0; index < items.length; index += 1) {
    const item = items[index];
    if (item === undefined) continue;
    const isTool = item.item.kind.type === "toolCall" || item.item.kind.type === "toolResult";
    if (!isTool) { entries.push({ kind: "message", item }); continue; }
    const activity: NoteTimelineItem[] = [];
    while (index < items.length) {
      const next = items[index];
      if (next === undefined || (next.item.kind.type !== "toolCall" && next.item.kind.type !== "toolResult")) break;
      activity.push(next);
      index += 1;
    }
    index -= 1;
    entries.push({ kind: "activity", items: activity });
  }
  return entries;
}

function noteActivityLabel(items: readonly NoteTimelineItem[]): string {
  const first = items[0]?.item.kind;
  const name = first?.type === "toolCall" ? first.name : first?.type === "toolResult" ? "工具结果" : "工具";
  const steps = items.filter(({ item }) => item.kind.type === "toolCall").length || items.length;
  const error = items.some(({ item }) => item.kind.type === "toolResult" && item.kind.isError === true);
  const active = items.some(({ item }) => !item.completed && !item.interrupted);
  const state = error ? "工具失败" : active ? "工作中" : "已完成";
  return `${state} · ${name}${steps > 1 ? ` · ${steps} 步` : ""}`;
}
export function NoteAgentPanel(props:Props):React.JSX.Element {return <NoteAgentWorkspace key={props.snapshot.page.id} kind="page" {...props}/>;}
export function ReadingNoteAgentPanel(props:{readonly resourceGateways?:AgentResourceGateways;readonly note:ReadingNoteBlock;readonly isSaved:boolean;readonly autoDispatch?:boolean}):React.JSX.Element {
  return <NoteAgentWorkspace key={props.note.id} kind="reading" {...props}/>;
}
/** Hiding retains the selected session; changing source creates a new host. */
function NoteAgentWorkspace(props:Host):React.JSX.Element {
  const host=identity(props);
  const binding={kind:host.bindingKind,resourceId:host.id} as const;
  const lifecycleRef=useAgentHostLifecycle([host.bindingKind,host.id]);
  const [open,setOpen] = useState(false);
  const [selected,setSelected] = useState<string | null>(() => readAgentSessionSelection(binding) ?? pendingBoundConversationId(binding));
  const [pendingConversationId,setPendingConversationId] = useState<string | null>(() => readAgentSessionSelection(binding) ? null : pendingBoundConversationId(binding));
  const [busy,setBusy] = useState(false);
  const [refresh,setRefresh] = useState(0);
  const [searchHit,setSearchHit] = useState<AgentSearchHit | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const selectedRef = useRef(selected);
  const pendingConversationRef = useRef(pendingConversationId);
  selectedRef.current = selected;
  pendingConversationRef.current = pendingConversationId;
  const { tasks: childTasks } = useAgentChildTasks(selected, refresh);
  function hide(): void { setOpen(false); queueMicrotask(() => trigger.current?.focus()); }
  function handleConversationCreated(conversation: import("@research-assistant/core/agent").AgentConversationSummary): void {
    // A pending request may finish after the user has selected another
    // conversation.  Refresh the list, but do not clear the new pending
    // state or move its selection back to the old conversation.
    setRefresh(value=>value+1);
    if (selectedRef.current !== conversation.id || pendingConversationRef.current !== conversation.id) return;
    setPendingConversationId(null);
    writeAgentSessionSelection(binding,conversation.id);
  }
  async function resumeChildTask(childConversationId: string): Promise<void> {
    if (!selected || busy) return;
    const epoch=lifecycleRef.current;
    setBusy(true);
    try {
      if (props.kind === "page") await agentSubtaskGateway.resumeNote(selected, childConversationId, host.id);
      else await agentSubtaskGateway.resumeReadingNote(selected, childConversationId, host.id);
      if (lifecycleRef.current!==epoch) return;
      notify.success(`已恢复${props.kind === "page" ? "笔记" : "阅读笔记"}子任务，正在继续执行。`);
      setRefresh(value => value + 1);
    } catch {
      if (lifecycleRef.current!==epoch) return;
      notify.error(`无法恢复${props.kind === "page" ? "笔记" : "阅读笔记"}子任务：会话绑定或冻结提交已不可用。`);
    } finally { if (lifecycleRef.current===epoch) setBusy(false); }
  }
  return <>
    <ControlHint content={host.label}><button ref={trigger} className="note-agent-trigger" type="button" aria-label={`打开 ${host.label}`}  aria-expanded={open} onClick={() => setOpen(true)}><MessageCircle size={22} aria-hidden="true" /></button></ControlHint>
    {open || selected ? <section hidden={!open} className="note-agent-panel" role="dialog" aria-modal="false" aria-label={host.label}>
      <header><strong>{host.label} · {host.title}</strong><ControlHint content="隐藏助手，任务继续运行"><button type="button" aria-label={`隐藏 ${host.label}`}  onClick={hide}><X size={18} /></button></ControlHint></header>
      <div className="note-agent-body">
        <aside inert={busy} aria-label="笔记对话">
          <BoundConversationList binding={binding} pendingConversationId={pendingConversationId} selectedId={selected}
            onSelect={(id,_archived,hit,pending) => {if (pending) {setPendingConversationId(id);clearAgentSessionSelection(binding);} else {setPendingConversationId(null);writeAgentSessionSelection(binding,id);}setSelected(id);setSearchHit(hit ?? null);setRefresh(value=>value+1);}}
            onRefreshSelected={()=>setRefresh(value=>value+1)} onArchiveChange={()=>setRefresh(value=>value+1)}
            onDeleted={id=>{if (id===selected) {clearAgentSessionSelection(binding,id);setSelected(null);}}} />
          {childTasks.length > 0 && <section aria-label="子任务" className="note-agent-child-tasks"><strong>子任务</strong>{childTasks.map((task) => <div key={task.childConversationId}><button disabled={busy} onClick={() => { setSelected(task.childConversationId); setRefresh(value => value + 1); }} type="button"><span>深度 {task.depth} · {task.status}</span><small>{task.modelCalls} 次模型 / {task.toolCalls} 次工具预算</small></button>{task.status === "recovery_required" && <ControlHint content={`使用冻结提交恢复${props.kind === "page" ? "笔记" : "阅读笔记"}子任务`}><button disabled={busy} onClick={() => void resumeChildTask(task.childConversationId)} type="button" >恢复并继续</button></ControlHint>}</div>)}</section>}
        </aside>
        {selected ? <NoteAgentSession key={selected} {...props} conversationId={selected} pendingConversation={pendingConversationId === selected} onConversationCreated={handleConversationCreated}
          onCreationFinished={() => {if(selectedRef.current===selected && pendingConversationRef.current===selected){setPendingConversationId(null);setSelected(null);}}} refresh={refresh} searchHit={searchHit} open={open} onBusy={setBusy} onHide={hide} />
          : <p className="note-agent-empty">新建或选择对话，围绕当前笔记提问。</p>}
      </div>
    </section> : null}
  </>;
}

function NoteAgentSession(props:Host & {
  readonly conversationId:string; readonly pendingConversation?: boolean; readonly onConversationCreated?: (conversation:import("@research-assistant/core/agent").AgentConversationSummary)=>void; readonly onCreationFinished:()=>void; readonly refresh:number; readonly searchHit:AgentSearchHit|null; readonly open:boolean; readonly onBusy:(value:boolean)=>void; readonly onHide:()=>void;
}):React.JSX.Element {
  const {conversationId,refresh,searchHit,open,onBusy,onHide}=props;
  const pendingConversation = props.pendingConversation === true;
  const host=identity(props);
  const store=useAgentDraftStore();
  const { creation, ensureConversation, captureAdoption } = useNoteConversationCreation({ binding: {kind: host.bindingKind, resourceId: host.id}, conversationId,
    pending: pendingConversation, saved: host.saved, store, ...(props.onConversationCreated ? {onCreated: props.onConversationCreated} : {}) });
  const [draft,setDraft,captureAcknowledgment,draftResources]=useConversationDraft(store,{hostKind:host.bindingKind,hostId:host.id,conversationId:pendingConversation?null:conversationId});
  const activation=useMemo(()=>({refresh}),[refresh]);
  const durableConversationId = pendingConversation ? null : conversationId;
  const history=useAgentConversation(durableConversationId, pendingConversation ? null : activation);
  const metadata=useConversationMetadata(durableConversationId,{kind:host.bindingKind,resourceId:host.id});
  const queueScope=useMemo(()=>({conversationId,scope:{kind:host.bindingKind,resourceId:host.id}}),[conversationId,host.bindingKind,host.id]);
  const queueReady=!pendingConversation&&metadata.status==="ready"&&!metadata.conversation?.archived&&history.status==="ready";
  const queued=useNoteInputQueue(pendingConversation?null:queueScope,queueReady,refresh);
  const queueState=queued.submission;
  const [state,setState]=useState<NoteSubmissionSnapshot | ReadingNoteSubmissionSnapshot | null>(null);
  const controller=useRef<NoteSubmissionController | ReadingNoteSubmissionController | null>(null);
  const input=useRef<AgentComposerInputHandle>(null);
  const resourceScope={conversationId, kind:host.bindingKind, resourceId:host.id};
  const inputResources=useAgentComposerInput({text:draft,setText:setDraft,input,access:draftResources,scope:resourceScope,onCommand:submitCommand,onCommandFailure:()=>pauseRejectedSteerQueue(queued.acquireOwner()),...(props.resourceGateways?{gateways:props.resourceGateways}:{})});
  const messages=useRef<HTMLDivElement>(null);
  const revealed=useRef<object | null>(null);
  const [selection,setSelection]=useState("");
  const [model,setModel]=useState<LlmModelSelection | undefined>();
  const [speech,setSpeech]=useState<SpeechInputState>("idle");
  const foreignStop=useRef<string | null>(null);
  const wasOpen=useRef(false);
  const [foreignStopping,setForeignStopping]=useState<string | null>(null);
  const autoDispatchedTurn=useRef<string | null>(null);
  useEffect(()=>{if(!pendingConversation)void metadata.refresh();},[refresh,pendingConversation]);
  useEffect(()=>{if(open&&wasOpen.current)history.reconnect?.();wasOpen.current=open;},[open,history.reconnect]);
  useEffect(()=>{
    const ownerKey=props.kind==="page"
      ? noteSubmissionOwnerKey({conversationId,pageId:host.id})
      : readingNoteSubmissionOwnerKey({conversationId,readingNoteId:host.id});
    const lease=props.kind==="page"
      ? noteSubmissionOwners.acquire(ownerKey,()=>new NoteSubmissionController({conversationId,pageId:host.id},agentNoteSubmissionGateway,agentTurnGateway))
      : readingNoteSubmissionOwners.acquire(ownerKey,()=>new ReadingNoteSubmissionController({conversationId,readingNoteId:host.id},agentReadingNoteSubmissionGateway,agentTurnGateway));
    const owned=lease.controller;
    controller.current=owned;setState(owned.getSnapshot());
    const unsubscribe=owned.subscribe(()=>setState(owned.getSnapshot()));
    return ()=>{unsubscribe();lease.release();controller.current=null;};
  },[conversationId,host.id,props.kind]);
  useEffect(()=>{if(open && !searchHit?.itemId) input.current?.focus();},[open,searchHit]);
  useEffect(()=>{
    if(!open || history.status!=="ready" || revealed.current===activation || !searchHit?.itemId) return;
    revealed.current=activation;
    const target=Array.from(messages.current?.querySelectorAll<HTMLElement>("article[data-item-id]") ?? [])
      .find(item=>item.dataset.itemId===searchHit.itemId && item.dataset.turnId===searchHit.turnId);
    if(target){target.scrollIntoView({block:"center"});target.focus({preventScroll:true});}
    else notify.error("未能找到这条历史消息，请刷新对话列表后重试。");
  },[open,history.status,history.projection,activation,searchHit]);
  useEffect(()=>{
    if(history.status!=="ready") return;
    const turn=state?.preview && history.projection?.turns.get(state.preview.turnId);
    if(turn?.outcome && state?.preview) controller.current?.observeOutcome(state.preview.turnId,turn.outcome);
  },[history.status,history.projection,state?.preview,state?.phase]);
  useEffect(()=>{if(state?.phase==="started") setDraft(value=>value.trim()===state.request?.text ? "" : value);},[state?.phase,state?.request,setDraft]);
  const regularLocked=state?.phase==="preparing" || state?.phase==="preview" || state?.phase==="starting" || (state?.error==="start" && !state.outcome);
  const queueLocked=queueState?.phase==="preparing"||queueState?.phase==="preview"||queueState?.phase==="starting"||(queueState?.error==="start"&&!queueState.outcome)||!!queueState?.cancelPending||queueState?.cancelError==="uncertain";
  const locked=regularLocked||queueLocked;
  const preparingInputId=queued.state?.queue?.preparingInputId ?? queueState?.request?.inputId;
  const ownsPreparingInput=!!preparingInputId && (queueState?.request?.inputId===preparingInputId || queueState?.preview?.confirmation.inputId===preparingInputId);
  const queueMode=queueState?.preview ? executionModeForModel(queueState.preview.input.model,queueState.preview.input.reasoningEffort) : null;
  const queuePreparation = !preparingInputId ? "none" : ownsPreparingInput && queueState?.phase==="started" && !!queueState.admission && !queueState.error ? "sent"
    : ownsPreparingInput && !queueState?.error && (queueState?.phase==="preparing" || queueState?.phase==="starting"
      || queueState?.phase==="preview" && queueState.preview?.kind==="pending" && !!queueMode) ? "sending" : "recovery";
  const queuePreparing = queuePreparation==="sending" || queuePreparation==="recovery";
  const canRecoverQueue=queueState?.phase==="idle" || queueState?.error==="prepare" || queueState?.error==="start" && !!queueMode
    || queueState?.error==="startRejected" || queueState?.cancelError==="rejected" || queueState?.cancelError==="uncertain";
  async function recoverQueuePreparation(expectedInputId:string):Promise<void> {
    const owner=queued.review.current, current=owner?.getSnapshot();
    if (!owner || !current) return;
    const currentInputId=queued.edits.current?.getSnapshot().queue?.preparingInputId ?? current.request?.inputId ?? current.preview?.confirmation.inputId;
    if (currentInputId!==expectedInputId || current.request?.inputId && current.request.inputId!==expectedInputId
      || current.preview?.confirmation.inputId && current.preview.confirmation.inputId!==expectedInputId) return;
    if (current.cancelError==="uncertain") await owner.cancel(crypto.randomUUID());
    else if (current.phase==="idle") await owner.recoverPending();
    else if (current.error==="prepare") await owner.retryPrepare(true);
    else if (current.error==="start" && current.preview) {
      const mode=executionModeForModel(current.preview.input.model,current.preview.input.reasoningEffort);
      if(mode) await owner.confirm(mode);
    } else if (current.error==="startRejected" || current.cancelError==="rejected") await owner.refreshPending();
    await queued.edits.current?.reload();
  }
  const queueTurn=queueState?.preview&&!queueState.outcome&&(queueState.phase==="starting"||queueState.phase==="started"||queueState.error==="start")?queueState.preview.turnId:null;
  const activeTurn=history.projection?.activeTurn ?? queueTurn ?? (state?.phase==="started" && !state.outcome ? state.admission?.turnId : null);
  const toolRecoveryBlocked=activeTurn===null && !!history.projection && agentConversationRequiresToolRecovery(history.projection);
  const steering=useNoteSteering(queueReady&&activeTurn?{...queueScope,turnId:activeTurn}:null,text=>setDraft(value=>value.trim()===text?"":value),history.status==="ready"?history.projection:null,{key:JSON.stringify([host.bindingKind,host.id,conversationId]),conversationId,captureAcknowledgment,queueRejected:(request,ack)=>recoverRejectedSteer(queued.acquireOwner(),request,ack??(()=>undefined))},{state:queued.state,isReady:()=>{const queue=queued.edits.current?.getSnapshot();return !!queue?.queue&&!queue.loading&&!queue.editing&&!queue.uncertain&&!queue.error;}});
  useEffect(()=>{onBusy(!!locked||steering.unresolved||!!queued.state?.editing||!!queued.state?.uncertain);return ()=>onBusy(false);},[locked,steering.unresolved,onBusy,queued.state?.editing,queued.state?.uncertain]);
  useEffect(()=>{if(foreignStop.current !== activeTurn){foreignStop.current=null;setForeignStopping(null);}},[activeTurn]);
  useEffect(()=>{
    if(history.status!=="ready") return;
    const turn=queueState?.preview&&history.projection?.turns.get(queueState.preview.turnId);
    if(turn?.outcome&&queueState?.preview)queued.review.current?.observeOutcome(queueState.preview.turnId,turn.outcome);
  },[history.status,history.projection,queueState?.preview,queueState?.phase]);
  useEffect(()=>{if(queueState?.phase==="started"||queueState?.outcome)void queued.edits.current?.reload();},[queueState?.phase,queueState?.outcome]);
  const reason=pendingConversation ? (!host.saved ? "请等待笔记保存完成或解决保存冲突后再准备来源。" : undefined)
    : metadata.status!=="ready" ? "正在核对笔记对话状态；读取失败时请刷新列表。"
    : metadata.conversation?.archived ? "已归档的对话只能阅读。"
    : history.status!=="ready" ? "正在读取对话历史；读取失败时请重新选择对话。"
    : toolRecoveryBlocked ? "工具操作尚待核对，请勿重复提交。"
    : !host.saved ? "请等待笔记保存完成或解决保存冲突后再准备来源。" : undefined;
  const choices=useMemo(()=>props.kind==="page"?blocks(props.snapshot):[],[props.kind==="page"?props.snapshot.document:null,host.id]);
  const missing=!!selection && !choices.some(item=>item.id===selection);
  async function stop(): Promise<void> {
    if(queueState?.preview&&(activeTurn===queueState.preview.turnId||queueState.phase==="starting"||queueState.error==="start")) {await queued.review.current?.stop();await queued.edits.current?.reload();return;}
    if(state?.preview && (activeTurn===state.preview.turnId || state.phase==="starting" || state.error==="start")) {await controller.current?.stop();await queued.edits.current?.reload();return;}
    if(!activeTurn || foreignStop.current===activeTurn) return;
    const target=activeTurn;
    foreignStop.current=target;setForeignStopping(target);
    try {
      const result=await agentTurnGateway.interrupt({conversationId,turnId:target});
      await queued.edits.current?.reload();
      if(!result.cancellationRequested && foreignStop.current===target){foreignStop.current=null;setForeignStopping(null);}
    } catch {
      if(foreignStop.current===target){foreignStop.current=null;setForeignStopping(null);}
      notify.error("无法确认停止结果，请查看历史后重试。");
    }
  }
  const supported=state?.preview?.input.model;
  const mode=supported === undefined ? null : executionModeForModel(supported, state?.preview?.input.reasoningEffort);
  const autoDispatch = props.autoDispatch !== false;
  useEffect(() => {
    const preview = state?.preview;
    if (toolRecoveryBlocked || (!pendingConversation && history.status !== "ready") || !autoDispatch || !preview || state?.phase !== "preview" || state.error || !mode || metadata.status !== "ready" || metadata.conversation?.archived || autoDispatchedTurn.current === preview.turnId) return;
    autoDispatchedTurn.current = preview.turnId;
    void controller.current?.confirm(mode);
  }, [toolRecoveryBlocked, pendingConversation, history.status, autoDispatch, metadata.conversation?.archived, metadata.status, mode, state?.error, state?.phase, state?.preview?.turnId]);
  const canPrepare=!queuePreparing && !inputResources.commandPending && !steering.unresolved && (!reason || pendingConversation) && !activeTurn && !locked && !queued.state?.uncertain && !queued.state?.editing
    && (pendingConversation || (queueState?.phase==="idle" && !queued.state?.queue?.preparingInputId && state?.phase==="idle"))
    && !!draft.trim() && !missing && speech==="idle";
  const queueFull=(queued.state?.queue?.inputs.length??0)>=64;
  const queueTooLong=new TextEncoder().encode(draft.trim()).byteLength>65536;
  const canEnqueue=!queuePreparing&&!steering.unresolved&&queueReady&&!locked&&!queued.state?.loading&&!queued.state?.editing&&!queued.state?.uncertain&&!!queued.state?.queue&&!queueFull&&!queueTooLong&&!!draft.trim()&&speech==="idle"&&(!draftResources.draft().queuePlacement||!queued.state.queue.preparingInputId);
  const canSteer=!queuePreparing&&!!activeTurn&&!inputResources.commandPending&&queueReady&&!locked&&!steering.unresolved&&!queued.state?.editing&&!queued.state?.uncertain&&!queueTooLong&&!!draft.trim()&&speech==="idle"&&!state?.stopRequested&&!state?.stopPending&&!queueState?.stopRequested&&!queueState?.stopPending&&(!activeTurn||foreignStopping!==activeTurn);
  const canSteerQueued=!queuePreparing&&(!!activeTurn||inputResources.commandPending)&&queueReady&&!locked&&!steering.unresolved&&!steering.state.awaitingHistory?.length&&!steering.pending&&!queued.state?.editing&&!queued.state?.uncertain&&!state?.stopRequested&&!state?.stopPending&&!queueState?.stopRequested&&!queueState?.stopPending&&(!activeTurn||foreignStopping!==activeTurn);
  const running=!!activeTurn || queueState?.phase==="started" || state?.phase==="started";
  const autoPrepare=useAgentQueueAutoPrepare(
    running,
    queued.state?.queue?.inputs.find(input => !input.paused) ?? null,
    inputResources.commandPending || !queueReady || !!reason || missing || locked || !!queued.state?.loading || !!queued.state?.editing || !!queued.state?.uncertain || !!queued.state?.queue?.paused || !!queued.state?.queue?.preparingInputId || queueState?.phase!=="idle" || state?.phase!=="idle",
    () => { const live=queued.edits.current?.getSnapshot();if(!live||live.loading||live.editing||live.uncertain||live.queue?.preparingInputId)return false; void prepareQueued(); },
  );
  function sendInput():void{if(queuePreparing||inputResources.consumeCommand())return;if(draftResources.draft().queuePlacement||inputResources.commandPending){void enqueue();}else if(activeTurn){if(canSteer){const submitted=inputResources.submission();void steering.submit(submitted.text,{userInput:submitted.userInput});}}else void prepare();}
  function submitCommand(text: string, intent: AgentCommandIntent): void {
    if (!queueReady || reason || locked || queuePreparing || steering.unresolved) return;
    if ((activeTurn || inputResources.commandPending) && intent === "steer") { notify.error(compactRunningMessage); return; }
    if (activeTurn || inputResources.commandPending || draftResources.draft().queuePlacement) void enqueue(text);
    else void inputResources.executeCommand(draft);
  }
  async function enqueue(commandText?: string):Promise<void> {
    if(!canEnqueue)return;
    const submitted=inputResources.submission();
    const owned=queued.edits.current,text=commandText ?? submitted.text;if(!owned)return;
    const current=owned.getSnapshot().queue;if(!current)return;
    if(draftResources.draft().queuePlacement&&current.preparingInputId)return;
    const applied=await owned.edit(queueDraftEdit(draftResources.draft(),current,text,commandText ? [] : submitted.userInput),captureAcknowledgment(commandText ? draft : text));
    if(applied)autoPrepare.markQueued();
  }
  async function steerQueuedInput(inputId: string, text: string): Promise<boolean> {
    if (inputResources.commandPending || !canSteerQueued) return false;
    if (readComposerCommand(text)) return await dispatchQueuedAgentCommand({inputId, text, running: !!activeTurn || inputResources.commandPending, execution: {compact: inputResources.executeCommand},
      transfer: () => queuedCommandTransfer(queued.edits, inputId, text, () => queued.retainTransfer({type: "remove", inputId}))}) ?? false;
    const transfer = queued.retainTransfer({ type: "remove", inputId });
    if (!transfer) return false;
    return steering.submit(text, {
      confirmed: () => transfer.apply(),
      rejected: transfer.release,
        queueRejected: async () => prioritizeRejectedSteerLease(queued.acquireOwner(),inputId),
      userInput: queued.state?.queue?.inputs.find(input=>input.inputId===inputId)?.userInput ?? [],
    });
  }
  async function retryQueue():Promise<void>{
    await queued.edits.current?.retry();
  }
  async function prepareQueued():Promise<void> {
    const head=queued.state?.queue?.inputs.find(input=>!input.paused),queue=queued.state?.queue;
    if(inputResources.commandPending||!!steering.state.awaitingHistory?.length||steering.unresolved||!head||!queue||queue.paused||queue.preparingInputId||reason||missing||activeTurn||locked||state?.phase!=="idle"||queueState?.phase!=="idle")return;
    if (readComposerCommand(head.text)) {
      await dispatchQueuedAgentCommand({inputId: head.inputId, text: head.text, running: !!activeTurn, execution: {compact: inputResources.executeCommand},
        transfer: () => queuedCommandTransfer(queued.edits, head.inputId, head.text, () => queued.retainTransfer({type: "remove", inputId: head.inputId}))});
      return;
    }
    const review = queued.review.current;
    const base={conversationId,requestId:crypto.randomUUID(),text:head.text,userInput:head.userInput ?? [],...(model?{model}:{})};
    const submission=props.kind==="page"
      ?{kind:"note" as const,request:{...base,context:{pageId:host.id,expectedRevision:host.revision,blockId:selection||null}}}
      :{kind:"readingNote" as const,request:{...base,context:{readingNoteId:host.id,expectedRevision:host.revision}}};
    try {
      const runtime = getAppRuntime();
      await freezeNewPreviewOwnerIntent(runtime.nativeAgentPreview, runtime.storage, conversationId, base.requestId);
    } catch (cause) {notify.error(`无法保存预览请求归属，队列消息已保留。（${cause instanceof Error ? cause.message : "agent_preview_owner_intent_failed"}）`);return;}
    if (queued.review.current !== review) return;
    try {
      await review?.prepare({inputId:head.inputId,expectedRevision:queue.revision,submission});
      const prepared = review?.getSnapshot();
      const preparedModel = prepared?.preview?.input.model;
      const preparedMode = preparedModel === undefined ? null : executionModeForModel(preparedModel, prepared?.preview?.input.reasoningEffort);
      if (prepared?.phase === "preview" && prepared.preview && !prepared.error && preparedMode
        && metadata.status === "ready" && !metadata.conversation?.archived) {
        await review?.confirm(preparedMode);
      }
    } catch {
      notify.error("无法准备队首来源，消息已保留。请检查来源后重试。");
    }
  }
  async function prepare(): Promise<void> {
    if(!canPrepare) return;
    const owned=controller.current;if(!owned)return;
    const submitted=inputResources.submission();
    const userInput=submitted.userInput;
    const base={conversationId,requestId:crypto.randomUUID(),text:submitted.text,userInput,...(model?{model}:{})};
    try {
      const runtime = getAppRuntime();
      await freezeNewPreviewOwnerIntent(runtime.nativeAgentPreview, runtime.storage, conversationId, base.requestId);
    } catch (cause) {notify.error(`无法保存预览请求归属，草稿已保留。（${cause instanceof Error ? cause.message : "agent_preview_owner_intent_failed"}）`);return;}
    if (controller.current !== owned) return;
    if (!(await ensureConversation())) return;
    if (controller.current !== owned) return;
    const acknowledgment=captureAcknowledgment(base.text);
    const pending=owned instanceof ReadingNoteSubmissionController
      ? owned.prepare({...base,context:{readingNoteId:host.id,expectedRevision:host.revision}}, acknowledgment)
      : owned.prepare({...base,context:{pageId:host.id,expectedRevision:host.revision,blockId:selection||null}}, acknowledgment);
    void pending.catch(()=>notify.error("无法准备当前笔记来源，草稿已保留。"));
  }
  return <div className="note-agent-session" onKeyDown={event=>{
    if(event.key!=="Escape" || event.nativeEvent.isComposing || event.defaultPrevented || document.querySelector('dialog[open], [aria-modal="true"]')) return;
    event.preventDefault();event.stopPropagation();if(activeTurn) void stop();else onHide();
  }}>
    <div ref={messages} className="note-agent-messages" aria-live="polite">
      <ConversationCreationRecovery {...creation} captureAdoption={captureAdoption} onFinished={props.onCreationFinished}/>
      <AgentSteeringReceipt awaitingHistory={steering.state.awaitingHistory} transferFailures={steering.state.transferFailures} onRetryTransfer={id=>void reconcileSteeringTransfer(queued.edits.current,id,steering.retryConfirmation)} uncertain={steering.uncertain} pending={steering.pending} text={steering.state.request?.text} status={steering.status} {...(steering.state.request?.inputId ? { requestId: steering.state.request.inputId } : {})} disabled={!queueReady} onRetry={()=>void steering.retry()}/>
      {!pendingConversation&&<NoteUnstartedSubmissions conversationId={conversationId}
        {...(props.kind==="page" ? {pageId:host.id,onReview:(receipt:import("@research-assistant/core/agent").AgentNoteReceipt)=>{
          const owned=controller.current;if(owned instanceof NoteSubmissionController)void owned.recover(receipt).catch(()=>notify.error("无法恢复原提交，请刷新后重试。"));
        }} : {readingNoteId:host.id,onReview:(receipt:import("@research-assistant/core/agent").AgentReadingNoteReceipt)=>{
          const owned=controller.current;if(owned instanceof ReadingNoteSubmissionController)void owned.recover(receipt).catch(()=>notify.error("无法恢复原提交，请刷新后重试。"));
        }})}
        disabled={steering.unresolved || state?.phase!=="idle" || queueState?.phase!=="idle" || !!activeTurn || metadata.status!=="ready" || !!metadata.conversation?.archived || history.status!=="ready"}/>}
      {!pendingConversation&&<NoteUnstartedSubmissions conversationId={conversationId} queueScope={queueScope}
        onReview={(admission:AgentNoteQueueAdmission)=>void queued.review.current?.recoverAdmission(admission)}
        disabled={steering.unresolved||!queueReady||!!activeTurn||state?.phase!=="idle"||queueState?.phase!=="idle"}/>}
      {!pendingConversation&&queuePreparation!=="sending"&&queueState&&queued.review.current&&<NoteQueueReview state={queueState} controller={queued.review.current}
        disabled={!queueReady||!!(activeTurn&&activeTurn!==queueState.preview?.turnId)} onChanged={()=>void queued.edits.current?.reload()}/>}
      {history.projection?.turnOrder.flatMap(turnId=>{
        const turn=history.projection?.turns.get(turnId);if(!turn)return [];
        const timeline=groupNoteTimeline(turn.itemOrder.flatMap(id=>{
          const item=turn.items.get(id);if(!item||item.kind.type==="providerNative")return [];
          const kind=item.kind;
          const isRewrite=kind.type==="toolResult" && kind.isError===false && item.completed && !item.interrupted
            && [...turn.items.values()].some(call=>call.kind.type==="toolCall" && call.kind.callId===kind.callId && call.kind.name==="research_propose_note_rewrite" && call.completed && !call.interrupted);
          return [{id,item,isRewrite}];
        }));
        const renderItem=({id,item,isRewrite}:NoteTimelineItem):React.JSX.Element=><article key={`${turnId}/${id}`} data-item-id={id} data-turn-id={turnId} tabIndex={-1} data-role={item.kind.type}><small>{item.kind.type==="userMessage"?"你":item.kind.type==="toolCall"?`工具：${item.kind.name}`:item.kind.type==="toolResult"?`工具结果${item.kind.isError?"（失败）":""}`:"助手"}</small>
          {isRewrite && props.kind==="page" ? <NoteRewriteReview scope={{conversationId,pageId:host.id,turnId,itemId:id}} currentRevision={host.revision} isSaved={host.saved} disabled={metadata.status!=="ready" || !!metadata.conversation?.archived}
            {...(props.onAccept?{onAccept:props.onAccept}:{})} {...(props.onUndo?{onUndo:props.onUndo}:{})} acceptanceBlocked={props.acceptanceBlocked??false}/>
            : <AgentMarkdown value={item.text} streaming={!item.completed && !item.interrupted}/>}
          {item.interrupted && <p>未完成</p>}
          {item.kind.type === "toolResult" && <AgentTextChangeCard conversationId={conversationId} content={item.text} />}
          <AgentMessageData messages={[{ id: id, role: item.kind.type === "userMessage" ? "user" : "assistant", content: item.text,
            rawData: { turnId, itemId: id, ...item } }]} read={() => readMessageInspection({ conversationId, turnId, itemId: id })} className="note-agent-message-copy" />
          {(item.kind.type === "userMessage" || item.kind.type === "assistantMessage") && <AgentMessageCopyButton value={item.text} className="note-agent-message-copy" />}
        </article>;
        const rendered=timeline.flatMap(entry=>entry.kind==="activity"
          ? [<details className="note-agent-activity-group" key={`${turnId}/${entry.items[0]?.id ?? "activity"}`}><summary>{noteActivityLabel(entry.items)}</summary><div>{entry.items.map(renderItem)}</div></details>]
          : [renderItem(entry.item)]);
        if(agentTurnReconnecting(turn))rendered.push(<p key={`${turnId}/reconnecting`} role="status">正在重新连接…</p>);
        const receipts = agentTurnUnconfirmedReceipts(turn);
        if(receipts.length)rendered.push(<div className="note-agent-error" key={`${turnId}/receipt-recovery`} role="alert">工具结果的原操作回执尚未确认，不能继续此对话或回滚；请先核对原操作，不要重复执行。轮次：{turnId}；待确认调用：{receipts.join("、")}</div>);
        const failureReason=agentTurnFailureReason(turn);
        const children = history.projection?.activeTurn === null ? agentTurnUnconfirmedChildCreations(turn) : [];
        const matchingChild = children.find(child => isMatchingChildCreationFailure(failureReason, child, conversationId, turnId));
        if (history.projection) for (const child of children) rendered.push(<AgentChildCreationRecovery key={`${turnId}/${child.callId}/child-creation`} identity={{...child, conversationId, turnId, watermark: history.projection.seq}} {...(child === matchingChild ? {diagnosticText: agentFailureDiagnostics(conversationId, turnId, turn), failureStage: childCreationFailureStage(failureReason, child, conversationId, turnId)} : {})} />);
        const content=failureReason===undefined || matchingChild?rendered:[...rendered,<div className="note-agent-error" key={`${turnId}/failure`} role="alert"><p>Agent 请求失败：{failureReason}</p>{agentFailureHint(failureReason)&&<p>{agentFailureHint(failureReason)}</p>}</div>];
        return [<AgentTurnChangeSummary key={turnId}>{content}</AgentTurnChangeSummary>];
      })}
      {state?.phase==="preparing" && <p role="status">正在准备已保存的笔记来源…</p>}
      {state?.preview && state.phase!=="idle" && <div className="note-agent-review">
        <NoteFrozenSubmission input={state.preview.input}/>
        {(state.phase==="preview" || state.error==="start") && !state.outcome && (!autoDispatch || state.error==="start") && <button type="button" disabled={toolRecoveryBlocked || (!pendingConversation && history.status!=="ready") || !mode || metadata.status!=="ready" || !!metadata.conversation?.archived} onClick={()=>{if(mode)void controller.current?.confirm(mode);}}>{state.error==="start"?"查询或继续原轮次":"确认发送"}</button>}
        {autoDispatch && state.phase==="preview" && !state.error && <p role="status">{!pendingConversation && history.status!=="ready" ? "正在核对对话历史，已保留原提交。" : toolRecoveryBlocked ? "工具操作尚待核对，已保留原提交。" : "上下文已冻结，正在发送…"}</p>}
        {!mode && <p>当前模型尚未通过 Agent 工具调用验证，请返回选择受支持的模型。</p>}
      </div>}
      {state?.error && <p role="alert">{state.error==="prepare"?"来源或笔记指令准备未确认，草稿已保留；请检查已保存内容及应用 Agent 配置。":state.error==="start"?"启动结果未确认；请查询原轮次，不要另建消息重发。":state.error==="startRejected"?"本次发送已被拒绝，请返回检查模型配置、来源或会话状态。":"无法确认停止结果，请重试。"}</p>}
      {state?.error==="prepare" && <><button type="button" onClick={()=>void controller.current?.retryPrepare(true)}>恢复原回执</button><button type="button" onClick={()=>void controller.current?.retryPrepare()}>重试原请求</button></>}
      {state?.phase!=="idle" && state?.phase!=="preparing" && state?.phase!=="starting" && (!activeTurn || !!state?.outcome) && !(state?.error==="start"&&!state.outcome) && <button type="button" onClick={()=>controller.current?.dismiss()}>返回输入</button>}
      {(!state?.outcome && (state?.stopPending || state?.stopRequested)) || (activeTurn && foreignStopping===activeTurn) ? <p role="status">正在等待任务停止，最终状态以对话历史为准。</p> : null}
      {state?.outcome && <p role="status">本轮{state.outcome==="completed"?"已完成":state.outcome==="canceled"?"已停止":"未完成"}。</p>}
      <AgentCompactionActivity records={inputResources.commandActivities} />
    </div>
    {!pendingConversation && <SystemDocumentApprovalPanel conversationId={conversationId}/>}
    <form className="agent-reference-anchor" onSubmit={event=>{event.preventDefault();sendInput();}} onKeyDownCapture={event=>{
      if (event.target instanceof HTMLElement && event.target.closest(".annotation-comment-editor")) return;
      inputResources.historyKeyDown(event);
    }}>
      {!pendingConversation&&queued.state&&<AgentInputQueue state={queued.state} className="note-agent-queue"
        compaction={inputResources.commandActivities.find(item => item.state === "pending")}
        disabled={!!steering.state.awaitingHistory?.length||steering.unresolved||steering.pending||!queueReady||state?.phase!=="idle"||queueState?.phase!=="idle"}
        explanation="队列会在当前回合结束后自动发送；展开后可编辑、移除或调整顺序。"
        preparingStatus={queuePreparation} onEdit={edit=>queued.edits.current?.edit(edit)??Promise.resolve(false)} onReload={()=>queued.edits.current?.reload()}
        onEditInComposer={(input,index,current)=>{draftResources.restoreQueue(input,index,current);return queued.edits.current?.edit({type:"remove",inputId:input.inputId})??Promise.resolve(false);}}
        onRetry={()=>void retryQueue()}
        {...(canSteerQueued ? { onSteer: steerQueuedInput } : {})}/>}
      {inputResources.menu}{inputResources.warning && <p role="alert">{inputResources.warning}</p>}
      <AgentComposerInputState preparingText={queuePreparing ? queuePreparation==="sending" ? "发送中…" : "消息发送状态待确认" : null}
        {...(queuePreparation==="recovery" && preparingInputId && canRecoverQueue ? {action:{label:"处理消息",onClick:()=>void recoverQueuePreparation(preparingInputId),disabled:!queueReady || !!queued.state?.loading || !!queued.state?.editing || !!queued.state?.uncertain || !!queueState?.cancelPending || queueState?.phase==="preparing" || queueState?.phase==="starting"}} : {})}>
      <AgentComposerTextInput key={draftResources.draft().queuePlacement?.input.inputId ?? "new"} inputRef={input} aria-label={`给 ${host.label}发送消息`} disabled={queuePreparing} placeholder="询问当前笔记，或描述希望整理的内容…" draft={inputResources.draft} onDraftChange={inputResources.changeDraft} onCaretChange={inputResources.update} onChange={inputResources.change} onKeyDown={event=>{
        if(inputResources.keyDown(event)||event.nativeEvent.isComposing||event.defaultPrevented||document.querySelector('dialog[open], [aria-modal="true"]'))return;
        if(inputResources.commandPending&&event.key==="Enter"&&(event.ctrlKey||event.metaKey)){event.preventDefault();return;}
        if(event.key==="Enter"&&!event.shiftKey){event.preventDefault();sendInput();}
      }}  {...(props.resourceGateways ? {gateways: props.resourceGateways} : {})} {...(resourceScope ? {scope: resourceScope} : {})} />
      </AgentComposerInputState>
      <p role="status">{steering.status || (!queueReady?reason:queueTooLong?"补充说明或排队消息最多 64 KB。":activeTurn?"Enter 补充说明 · 沿用本轮模型与冻结资料":reason) || (queueFull?"队列已满，请先处理已有消息。":missing?"所选块已不存在，请重新选择。":"Enter 准备 · Shift+Enter 换行")}</p>
      <div className="note-agent-tools">
        {props.kind==="page" ? <select aria-label="笔记来源范围" disabled={!!locked||!!activeTurn} value={selection} onChange={event=>setSelection(event.target.value)}><option value="">当前整页</option>{missing&&<option value={selection}>所选块已不存在</option>}{choices.map(item=><option key={item.id} value={item.id}>{item.label}</option>)}</select> : <ControlHint content="仅发送已保存批注及保留的来源摘录，不自动附加全文"><span>当前阅读笔记</span></ControlHint>}
        <AgentModelControl route="note_agent" label="选择 AI 笔记模型" disabled={!!locked||!!activeTurn} onSelection={setModel}/>
        <AgentResourcePickerMenu {...(props.resourceGateways?{gateways:props.resourceGateways}:{})} scope={resourceScope} onSelect={inputResources.select} disabled={queuePreparing || (!!locked && !activeTurn)} />
        <PaperAiReadingSpeechInput value={inputResources.value} onChange={inputResources.change} onStateChange={setSpeech} disabled={queuePreparing || !!locked||!!activeTurn}/>
        <ControlHint content="排队，作为下一轮消息"><button type="button" aria-label="将消息加入队列"  disabled={!canEnqueue} onClick={()=>void enqueue()}>排队</button></ControlHint>
        {activeTurn&&<ControlHint content="沿用本轮模型与已确认资料" shortcut="Enter"><button type="submit" disabled={draftResources.draft().queuePlacement?!canEnqueue:!canSteer} >{draftResources.draft().queuePlacement?"放回队列":"补充说明"}</button></ControlHint>}
        {activeTurn ? <button className="note-agent-send" type="button" aria-label="停止生成" onClick={()=>void stop()} disabled={state?.stopPending||state?.stopRequested||queueState?.stopPending||queueState?.stopRequested||foreignStopping===activeTurn}><Square size={14}/></button>
          : <button className="note-agent-send" type="submit" aria-label="准备发送消息" disabled={draftResources.draft().queuePlacement||inputResources.commandPending?!canEnqueue:!canPrepare}><ArrowUp size={16}/></button>}
      </div>
    </form>
  </div>;
}
