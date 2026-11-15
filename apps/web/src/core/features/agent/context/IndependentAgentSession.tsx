import { useNativeConversationContext } from "./useNativeConversationContext";
import { ContextConfigurationRecovery } from "./ContextConfigurationRecovery";
import { useAgentContextUsage } from "../useAgentContextUsage";
import { AgentSubmissionFeedback } from "../AgentSubmissionFeedback";
import { AgentRecoveryMessage } from "../AgentRecoveryMessage";
import { AGENT_START_FAILURE, isAgentStartRejection, recoverAgentPreparationFailure } from "../agentPreparationFailure";
import { notify } from "../../../services/notifications";
import { useNotificationState } from "../../../services/useNotificationState";
import type { LlmModelSelection } from "@research-assistant/core/ai";
import type { AgentCanvasMessage } from "../AgentCanvasMessage";
import { releaseRemovedEvidence } from "../evidenceImageGateway";
import { useConversationRetrieval } from "./useConversationRetrieval";
import { agentDraftFromInput, agentDraftInput, editAgentDraft, agentContextKey, serializeAgentSourceReferences, type AgentInputDraft } from "@research-assistant/core/agent";
import { contextComposerResources, turnContextResources } from "./contextComposerResources";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { freezeAgentContext, selectAgentContext, EMPTY_AGENT_CONTEXT, type AgentContextPreview, type AgentContextState } from "@research-assistant/core/agent";
import {useAgentContextCandidates, useAgentContextSuggestions} from "./AgentContextProvider";
import { useContextRecommendations } from "./useContextRecommendations";
import {agentContextGateway, type AgentContextGateway, type AgentContextSteerRequest} from "../../../services/agentContextGateway";
import { getAppStorage } from "../../../runtime/appStorage";
import { getAppRuntime } from "../../../runtime/appRuntime";
import { freezeNewPreviewOwnerIntent, NATIVE_PREVIEW_OWNER_CAPABILITY } from "../../../runtime/nativePreviewOwnerIntent";
import { WritingAgentPanel, type WritingAgentSubmitOptions } from "../presentation/WritingAgentPanel";
import { useAgentConversation } from "../useAgentConversation";
import { projectConversationMessages } from "../projectConversationMessages";
import { readConversationContext as readAgentContextState, subscribeConversationContext, writeConversationContext as writeAgentContextState } from "./contextStorage";
import type { AgentContextDirectory } from "./contextDirectoryGateway";
import { AgentContextControl } from "./AgentContextControl";
import { useCaptureAgentNavigation } from "../AgentCanvasNavigationGuard";
import { agentCatalogGateway, agentMetadataGateway } from "../../../services/agentGateway";
import { CONVERSATION_TITLE_CHANGED_EVENT, type ConversationTitleRenameAccess } from "../ConversationHeaderTitle";
import { nameInitialConversation } from "../automaticConversationName";
import {useConversationInputQueue} from "./useConversationInputQueue";
import {useAgentQueueAutoPrepare, type AgentQueueAutoPrepareInput} from "../useAgentQueueAutoPrepare";
import {WritingAgentTaskStack} from "../presentation/WritingAgentTaskStack";
import {IndependentPreparation} from "./independentPreparation";
import {independentQueueResume} from "./independentQueueResume";
import {decodeAgentSqlImmediateFailure} from "@research-assistant/core/agent";
import {diagnosticError} from "../../../services/diagnosticError";
import {useTaskObservation} from "./useTaskObservation";
import {useAgentSteering} from "../useAgentSteering";
import {useAgentSteeringOwners} from "../AgentSteeringProvider";
import {AgentSteeringReceipt} from "../AgentSteeringReceipt";
import {AgentPendingSteeringBubbles} from "../AgentPendingSteeringBubbles";
import {AgentPendingSubmissionBubble, hasConfirmedSubmissionMessage} from "../AgentPendingSubmissionBubble";
import {AgentSteeringHandoffRecovery} from "../AgentSteeringHandoffRecovery";
import {reconcileSteeringTransfer} from "../useAgentInputQueue";
import {recoverRejectedSteer} from "../rejectedSteerQueue";
import {useConversationSteeringHandoff} from "./useConversationSteeringHandoff";
import {useHiddenHandoffQueueInputs} from "./useHiddenHandoffQueueInputs";
import {independentQueuePreparingPresentation} from "../queuePreparingPresentation";
const DRAFT_ACKNOWLEDGED = "research-assistant:independent-steering-draft-acknowledged";
function message(error: unknown): string {
    const code = typeof error === "string" ? error : error instanceof Error ? error.message : "unavailable";
    return ({ paperNotParsed: "所选 PDF 尚无对应版本的 Docling 解析，请先解析后重试。", paperNotIndexed: "请先完成 PDF 解析后重试。", sourceUnavailable: "所选材料不可用或尚未保存，请检查后重试。", budget: "所选材料过大，请缩小范围。", modelUnavailable: "当前模型未配置或不支持独立对话，请重新选择。", conversationUnavailable: "此独立对话已不可用，请新建对话。", confirmationMismatch: "待发送快照身份不匹配，请重新准备。", unavailable: "暂时无法完成请求，请重试。", invalidInput: "消息或材料选择无效，请检查后重试。" } as Record<string, string>)[code] ?? code;
}
export function IndependentAgentSession({ messageRequest, id, title, gateway, open, onOpenChange, headerControls, onStarted, onBusyChange, archived = false, onDirectoryChanged,pending=false,ensureCreated=async()=>undefined, onRenamed, onRefreshTitle }: {
    readonly messageRequest?: AgentCanvasMessage | null;
    readonly onRenamed?: ConversationTitleRenameAccess["onRenamed"];
    readonly onRefreshTitle?: () => void;
    readonly pending?:boolean;
    readonly ensureCreated?:()=>Promise<void>;
    readonly archived?: boolean;
    readonly onDirectoryChanged?: (directory:AgentContextDirectory|null)=>void;
    readonly id: string;
    readonly title: string;
    readonly gateway: AgentContextGateway;
    readonly open: boolean;
    readonly onOpenChange: (open: boolean) => void;
    readonly headerControls: ReactNode;
    readonly onStarted: () => void;
    readonly onBusyChange?: (busy: boolean) => void;
}): React.JSX.Element {
    const captureDirectoryNavigation = useCaptureAgentNavigation();
    const storage = getAppStorage(), draftKey = `research-assistant:independent-draft:${id}`, pendingKey = `research-assistant:independent-pending:${id}`;
    const [initial] = useState(() => { try {
        return { context: readAgentContextState(id), error: undefined as string | undefined };
    }
    catch {
        return { context: EMPTY_AGENT_CONTEXT, error: "上下文配置无法读取，已停止自动恢复。请重新添加材料。" };
    } });
    const inputKey=`research-assistant:independent-input:${id}`, draftVersionKey=`research-assistant:independent-draft-version:${id}`;
    const guardedKey = `research-assistant:independent-preparation:v1:${id}`;
    const [, setGuardedRevision] = useState(0);
    const guarded = useMemo(() => gateway.preparation && gateway.guardedStart ? new IndependentPreparation(storage, guardedKey, id,
        gateway.preparation.status, gateway.preparation.proven, () => setGuardedRevision(value => value + 1), gateway.guardedStart) : null, [gateway, storage, guardedKey, id]);
    const guardedBlocked = !!guarded && guarded.getSnapshot().phase !== "idle";
    const inputDraft=useRef<AgentInputDraft>((()=>{const text=storage.getItem(draftKey)??"";try{return agentDraftFromInput(text,JSON.parse(storage.getItem(inputKey)??"[]"));}catch{return {text,bindings:[]};}})());
    const opened = useAgentContextSuggestions();
    const resources=useMemo(()=>contextComposerResources(gateway,undefined,opened),[gateway,opened]);
    const [context, setContext] = useState<AgentContextState>(initial.context), [draft, setDraft] = useState(() => storage.getItem(draftKey) ?? "");
    const recommendations = useContextRecommendations(id, context, pending);
    const [preview, setPreview] = useState<AgentContextPreview | null>(null), [busy, setBusy] = useState(false), [uncertain, setUncertain] = useState(false), [error, setError] = useNotificationState("error");
    const [pendingSubmission, setPendingSubmission] = useState<{id:string;conversationId:string;text:string;userInput:readonly import("@research-assistant/core/agent").AgentUserInput[];turnId?:string} | null>(null);
    useEffect(()=>subscribeConversationContext(id,()=>{try{setContext(readAgentContextState(id));}catch{setError("上下文配置无法读取，请重试。");}}),[id]);
    const nativeContext = useNativeConversationContext(id,pending,gateway,setContext,reason => setError(`无法恢复上下文：${message(reason)}`),{readOnly:archived});
    const candidates = useAgentContextCandidates(), candidate = candidates[0];
    useEffect(() => {
        if (!pending || !candidate || initial.error || storage.getItem(pendingKey) !== null || guardedBlocked) return;
        const autoKey = `research-assistant:context-auto:${id}`;
        const automatic = storage.getItem(autoKey), candidateKey = agentContextKey(candidate.resource);
        // An unsent automatic draft follows current defaults. Manual permissions,
        // deselections and an admitted/frozen request keep their saved authority.
        if (automatic === "manual" || (automatic !== null && automatic !== candidateKey)
            || (automatic === null && (context.persistent.length > 0 || context.temporary.length > 0))) return;
        try {
            const next = selectAgentContext(context, candidate, "persistent");
            if (JSON.stringify(next) === JSON.stringify(context)) return;
            writeAgentContextState(id, next);
            storage.setItem(autoKey, candidateKey);
            setContext(next);
        } catch {setError("无法保存推荐上下文，请重试。");}
    }, [pending, pendingKey, id, candidate, context, storage, initial.error, guardedBlocked]);
    const [recoveryBlocked, setRecoveryBlocked] = useState(() => storage.getItem(pendingKey) !== null), [recoveryAttempt, setRecoveryAttempt] = useState(0);
    useEffect(() => { onBusyChange?.(busy); return () => onBusyChange?.(false); }, [busy, onBusyChange]);
    const alive = useRef(true), operation = useRef(false);
    const queuedDispatchInput = useRef<string | null>(null);
    // Before a durable preparation is admitted we can stop the local preflight
    // without racing a native start. Later stages retain their recovery owner.
    const sendPreflight = useRef<{requestId: string; canceled: boolean} | null>(null);
    const [preflightSending, setPreflightSending] = useState(false);
    const [cancelSendingPending, setCancelSendingPending] = useState(false);
    const currentId = useRef(id); currentId.current = id;
    const currentTitle = useRef(title); currentTitle.current = title;
    const externalRequest = useRef<AgentCanvasMessage | null>(null);
    const preserveDraft = useRef(false);
    const failureDraft = useRef(inputDraft.current);
    const retryRef = useRef<() => Promise<void>>(async () => undefined);
    const lastRecoveryNotice = useRef<string | null>(null);
    const reconciledAdmission = useRef<string | null>(null);
    function notifyRecovery(text: string, requestId: string): void {
        lastRecoveryNotice.current = requestId;
        notify.error(text, {action: {label: "重新发送", onClick: () => {
            try {
                const saved = JSON.parse(storage.getItem(pendingKey) ?? "null");
                if (alive.current && saved?.conversationId === id && saved?.requestId === requestId) void retryRef.current();
            } catch { if (alive.current) setError("无法核对待发送记录，草稿已保留。"); }
        }}});
    }
    const [chatModel, setChatModel] = useState<LlmModelSelection | undefined>(undefined);
    const access = useMemo(() => ({ identity: "independent", gateway: gateway.sessions }), [gateway]);
    const history = useAgentConversation(pending?null:id, undefined, access), projection = history.projection;
    const contextUsage = useAgentContextUsage(projection, undefined, chatModel);
    const activeTurn = projection?.activeTurn ?? null;
    const [awaitingTurn, setAwaitingTurn] = useState<string | null>(null);
    const taskObservation = useTaskObservation(id, storage, gateway.taskObservation);
    const running = activeTurn !== null || awaitingTurn !== null;
    const [queueRefresh, setQueueRefresh] = useState(0);
    const [stopping, setStopping] = useState(false);
    const queue = useConversationInputQueue(gateway, id, !pending && history.status === "ready" && !archived, queueRefresh);
    const queueState = queue.state?.queue;
    const queueBlocked = !queueState || queue.state?.loading || queue.state?.editing || queue.state?.pending || queue.state?.uncertain || !!queue.state?.error;
    const steeringOwners=useAgentSteeringOwners();
    const sendSteer=useMemo(()=>(request:AgentContextSteerRequest)=>gateway.steer ? gateway.steer(request) : Promise.reject(new Error("STEERING_UNAVAILABLE")),[gateway]);
    function captureSteeringDraft(text:string):()=>void {
        const version=storage.getItem(draftVersionKey), raw=storage.getItem(draftKey), input=storage.getItem(inputKey);
        const original=structuredClone(inputDraft.current);
        if(serializeAgentSourceReferences(original).text.trim()!==text.trim())return ()=>undefined;
        return ()=>{
            if(storage.getItem(draftVersionKey)!==version||storage.getItem(draftKey)!==raw||storage.getItem(inputKey)!==input)return;
            storage.removeItem(draftKey);storage.removeItem(inputKey);storage.setItem(draftVersionKey,crypto.randomUUID());
            releaseRemovedEvidence(original.evidence);
            window.dispatchEvent(new CustomEvent(DRAFT_ACKNOWLEDGED,{detail:id}));
        };
    }
    useEffect(()=>{
        const changed=(event:Event)=>{if((event as CustomEvent<string>).detail!==id)return;const text=storage.getItem(draftKey)??"";
            try{inputDraft.current=agentDraftFromInput(text,JSON.parse(storage.getItem(inputKey)??"[]"));setDraft(text);}catch{setError("无法读取草稿，请重试。");}};
        window.addEventListener(DRAFT_ACKNOWLEDGED,changed);return()=>window.removeEventListener(DRAFT_ACKNOWLEDGED,changed);
    },[id,storage,draftKey,inputKey]);
    const steeringScope=activeTurn && history.status==="ready" && projection?.turns.get(activeTurn)?.taskKind!=="compact" ? {conversationId:id,turnId:activeTurn} : null;
    const registry=gateway.steeringOwner && gateway.steeringOwner===agentContextGateway.steeringOwner?steeringOwners?.conversations:undefined;
    const handoffRegistry=registry && gateway.steeringHandoff===agentContextGateway.steeringHandoff?steeringOwners?.handoffs:undefined;
    const handoff=useConversationSteeringHandoff(id,gateway,handoffRegistry,history.status==="ready"&&!archived&&!pending,projection,async()=>{
        const owner=queue.controller.current;if(!owner)return false;
        const before=owner.getSnapshot().readSequence??0;await owner.reload();const after=owner.getSnapshot();
        return queue.controller.current===owner && !after.loading && !after.error && !after.uncertain && (after.readSequence??0)>before;
    },{kind:"context"},queue.state?.readSequence??0);
    const handoffBlocked=!pending&&!!gateway.steer&&(!handoff.available||handoff.blocked);
    // A known accepted handoff may wait for the current model response without
    // preventing unrelated new input. Unknown/recovered transfers stay fenced.
    const handoffWaitingIntents=[...(handoff.state?.items.map(item=>item.intent)??[]),...(handoff.state?.intent?[handoff.state.intent]:[])];
    const handoffAccepted=handoff.state?.phase === "waiting" && handoffWaitingIntents.length>0
      && handoffWaitingIntents.every(intent=>intent.turnId===activeTurn && handoff.state?.acceptedOperationIds?.includes(intent.operationId))
      && handoff.state.items.every(item=>item.history==="unconfirmed");
    const handoffInputBlocked=handoffBlocked&&!handoffAccepted;
    const steering=useAgentSteering(steeringScope,steeringScope?`${id}/${activeTurn}`:"",sendSteer,()=>undefined,history.status==="ready"?projection:null,
      registry?{registry,hostKey:id,conversationId:id,captureAcknowledgment:captureSteeringDraft,queueRejected:(request,ack)=>recoverRejectedSteer(queue.acquireOwner(),request,ack??(()=>undefined))}:undefined,
      {state:queue.state,isReady:()=>{const value=queue.controller.current?.getSnapshot();return !!value?.queue&&!value.loading&&!value.editing&&!value.uncertain&&!value.error;}});
    const queueHead = queueState && !queueState.paused && !queueState.preparingInputId ? queueState.inputs.find(input => !input.paused) ?? null : null;
    useEffect(() => { if (!running && !pending) setQueueRefresh(value => value + 1); }, [running,pending]);
    useEffect(() => { if (awaitingTurn && projection?.turns.has(awaitingTurn))
        setAwaitingTurn(null); }, [projection, awaitingTurn]);
    useEffect(() => {
        alive.current = true;
        const stored = storage.getItem(pendingKey);
        if (stored) {
            setBusy(true);
            operation.current = true;
            void (async () => {
                try {
                    const saved = JSON.parse(stored) as {
                        conversationId: string;
                        requestId: string;
                        contextSnapshotId: string;
                        phase: string;
                        preserveDraft?: boolean;
                    };
                    if (saved.conversationId !== id || typeof saved.requestId !== "string" || typeof saved.contextSnapshotId !== "string" || !["prepared", "starting"].includes(saved.phase))
                        throw new Error("待发送会话身份无效");
                    const restored = await gateway.restore({ conversationId: id, requestId: saved.requestId });
                    if (restored.contextSnapshotId !== saved.contextSnapshotId)
                        throw new Error("待发送快照身份无效");
                    if (alive.current) {
                        preserveDraft.current = saved.preserveDraft === true;
                        setPreview(restored);
                        setRecoveryBlocked(false);
                        setUncertain(saved.phase === "starting");
                        setError(undefined);
                    }
                }
                catch (reason) {
                    if (alive.current)
                        setError(`无法恢复待发送请求：${message(reason)}`);
                }
                finally {
                    if (alive.current) {
                        setBusy(false);
                        operation.current = false;
                    }
                }
            })();
        }
        return () => { alive.current = false; };
    }, [id, pendingKey, storage, gateway, recoveryAttempt]);
    async function changeContext(next: AgentContextState): Promise<void> { try {
        await ensureCreated();
        await nativeContext.save(next);
        setError(undefined);
    }
    catch (reason) {
        setError(`无法保存上下文：${message(reason)}`);
        throw reason;
    } }
    function changeDraft(text: string): void { try {
        inputDraft.current=editAgentDraft(inputDraft.current,text);
        storage.setItem(draftVersionKey, crypto.randomUUID());
        storage.setItem(draftKey, text);
        storage.setItem(inputKey,JSON.stringify(agentDraftInput(inputDraft.current)));
        setDraft(text);
    }
    catch (reason) {
        setError(`无法保存草稿：${message(reason)}`);
    } }
    const retrieval=useConversationRetrieval((text,options,traceId,submissionRequestId)=>void prepare(text,options,traceId,false,undefined,undefined,submissionRequestId));
    const autoQueue = useAgentQueueAutoPrepare(running, queueHead,
        pending || archived || !nativeContext.ready || busy || stopping || recoveryBlocked || guardedBlocked || !!taskObservation.state || !!preview || retrieval.pending || !!queueBlocked || handoffBlocked || history.status !== "ready" || !!error || steering.unresolved || !!steering.state.awaitingHistory?.length,
        input => {
            if (operation.current) return false;
            void prepare(input.text, {contextScope:"manuscript",userInput:input.userInput ?? []}, undefined, false, input);
        });
    useEffect(() => {
        if (!queueState?.preparingInputId || !gateway.recoverQueue || preview || recoveryBlocked || guardedBlocked || handoffBlocked || taskObservation.state || operation.current || running || archived) return;
        operation.current = true; setBusy(true);
        void gateway.recoverQueue(id).then(frozen => {
            if (!alive.current) return;
            if (!frozen) throw new Error("无法恢复队首提交");
            preserveDraft.current = true;
            storage.setItem(pendingKey, JSON.stringify({conversationId:id,requestId:frozen.requestId,contextSnapshotId:frozen.contextSnapshotId,phase:"prepared",preserveDraft:true}));
            setPreview(frozen); setUncertain(false);
        }).catch(reason => {if (alive.current) setError(message(reason));})
          .finally(() => {if (alive.current) {operation.current=false;setBusy(false);}});
    }, [queueState?.preparingInputId, queueRefresh, gateway, preview, recoveryBlocked, guardedBlocked, handoffBlocked, taskObservation.state, running, archived, id]);
    function enqueue(text: string, userInput: readonly import("@research-assistant/core/agent").AgentUserInput[] = []): void {
        if (archived || (!pending && !nativeContext.ready) || busy || stopping || recoveryBlocked || guardedBlocked || taskObservation.blocked || preview || queueBlocked || handoffInputBlocked || steering.unresolved || !queue.controller.current) return;
        const original = structuredClone(inputDraft.current);
        const savedText = storage.getItem(draftKey), savedInput = storage.getItem(inputKey);
        void queue.controller.current.edit({type:"add", inputId:crypto.randomUUID(), text, userInput}, () => {
            if (storage.getItem(draftKey) !== savedText || storage.getItem(inputKey) !== savedInput) return;
            storage.removeItem(draftKey); storage.removeItem(inputKey);
            releaseRemovedEvidence(original.evidence);
            if (alive.current && JSON.stringify(inputDraft.current) === JSON.stringify(original)) {
                inputDraft.current = {text:"",bindings:[]}; setDraft("");
            }
        });
    }
    async function stop(): Promise<void> {
        if (!activeTurn || stopping) return;
        setStopping(true);
        try {
            await gateway.interrupt(id, activeTurn);
            await queue.controller.current?.reload();
            history.reconnect();
        } catch (reason) {if (alive.current) setError(message(reason));}
        finally {if (alive.current) setStopping(false);}
    }
    async function finishGuarded(): Promise<void> {
        if (!guarded || !alive.current) return;
        const started = guarded.getSnapshot().record?.start?.result;
        if (started && !(await (guarded.getSnapshot().ownerSettled ? taskObservation.clearSettled(started) : taskObservation.track(started)))) return;
        if (!alive.current || currentId.current !== id) return;
        await guarded.finish(async record => {
            const frozen = record.committed!.result, receipt = record.start!.result!;
            if (title === "新对话" && !(projection?.turnOrder.some(turn => turn !== frozen.turnId))) {
                void nameInitialConversation(id, frozen.requestId, frozen.input.text, {catalog: agentCatalogGateway, metadata: agentMetadataGateway})
                    .then(row => {if (!row) return; window.dispatchEvent(new Event(CONVERSATION_TITLE_CHANGED_EVENT));
                        if (alive.current && currentId.current === row.id) {if (currentTitle.current === "新对话" || currentTitle.current === row.title) onRenamed?.(row); onRefreshTitle?.();}})
                    .catch(() => {if (alive.current) onRefreshTitle?.();});
            }
            const saved = record.draft;
            const acceptedContext = record.preserveDraft && !frozen.queue ? context : recommendations.prepareCommit(frozen.requestId);
            const next = record.preserveDraft && !frozen.queue ? acceptedContext : {...acceptedContext, temporary: acceptedContext.temporary.filter(selection =>
                !record.intent.resources.some(resource => JSON.stringify(resource) === JSON.stringify(selection.resource)))};
            if (!record.preserveDraft || frozen.queue) {
                await nativeContext.save(next);
                if (!alive.current || currentId.current !== id) throw new Error("CONTEXT_FINISH_HOST_CLOSED");
                recommendations.completeCommit(frozen.requestId);
            }
            const consume = !record.preserveDraft && !!saved && saved.version !== null
                && storage.getItem(draftVersionKey) === saved.version && storage.getItem(draftKey) === saved.text && storage.getItem(inputKey) === saved.input;
            if (consume) {
                storage.removeItem(draftKey); storage.removeItem(inputKey); storage.removeItem(draftVersionKey);
                releaseRemovedEvidence(inputDraft.current.evidence); inputDraft.current = {text:"",bindings:[]}; setDraft("");
            }
            setQueueRefresh(value => value + 1);
            if (receipt.outcome === null) setAwaitingTurn(receipt.turnId);
            history.reconnect(); onStarted();
        });
        if (guarded.getSnapshot().phase === "idle") notify.dismiss(guardedKey);
    }
    function notifyGuarded(): void {
        if (!guarded || !alive.current) return;
        const snapshot = guarded.getSnapshot(), ready = snapshot.phase === "ready", failure = decodeAgentSqlImmediateFailure(snapshot.cause);
        const direct = snapshot.phase === "unconfirmed" && snapshot.cause && typeof snapshot.cause === "object" ? snapshot.cause as {phase?: string; safeCause?: string} : undefined;
        const safe = diagnosticError(snapshot.cause), code = failure?.code ?? safe.code ?? (snapshot.phase === "admitted" ? "CONTEXT_QUEUE_ADMITTED_START_UNCONFIRMED" : "CONTEXT_PREPARE_UNCONFIRMED");
        const detail = failure?.diagnostic ?? direct;
        const fields = [`code=${code}`, `C=${id}`, ...(snapshot.record ? [`R=${snapshot.record.intent.requestId}`] : []),
            ...(detail?.phase ? [`phase=${detail.phase}`] : []), ...(detail?.safeCause ? [`cause=${detail.safeCause}`] : []),
            ...(failure?.diagnostic?.sqliteCode ? [`sqlite=${failure.diagnostic.sqliteCode}`] : []),
            ...(failure?.diagnostic?.sqliteExtendedCode != null ? [`sqliteExtended=${failure.diagnostic.sqliteExtendedCode}`] : []),
            ...(typeof safe.httpStatus === "number" ? [`HTTP=${safe.httpStatus}`] : [])];
        const originalRequestId = snapshot.record?.intent.requestId;
        notify.error(ready ? "原请求已准备，确认后可继续发送。" : snapshot.phase === "resumePending" ? `队列恢复结果尚未确认，请核对原恢复操作。 [${fields.join("；")}]`
            : snapshot.phase === "admitted" ? `队列消息已接纳，启动尚未确认；可继续原请求。 [${fields.join("；")}]`
            : safe.code === "AGENT_TASK_IDENTITY_UNAVAILABLE" ? `原请求已启动，但当前无法观察原运行；已暂停新发送，草稿和原请求已保留。 [${fields.join("；")}]`
            : snapshot.cause instanceof Error && snapshot.cause.message === "CONTEXT_QUEUE_PAUSED_OR_UNAVAILABLE" ? `队列已暂停或状态不匹配，原请求已保留。 [${fields.join("；")}]`
            : `发送结果尚未确认，草稿已保留。请核对原请求。 [${fields.join("；")}]`, {
            id: guardedKey,
            action: {label: snapshot.phase === "resumePending" ? "核对队列恢复" : ready || snapshot.phase === "admitted" ? "继续发送" : "核对原请求", onClick: () => {
                if (!originalRequestId || guarded.getSnapshot().record?.intent.requestId !== originalRequestId) return;
                void inspectGuarded(ready || snapshot.phase === "admitted" || snapshot.phase === "resumePending");}},
        });
    }
    async function inspectGuarded(continueRequested = false): Promise<void> {
        if (!guarded || operation.current || archived || (taskObservation.state && continueRequested)) return;
        operation.current = true; setBusy(true);
        try {
            if (continueRequested && guarded.getSnapshot().phase === "ready") await guarded.start(!!guarded.getSnapshot().record?.committed?.result.queue);
            else if (continueRequested && gateway.queue && (guarded.getSnapshot().phase === "admitted" || guarded.getSnapshot().phase === "resumePending")) {
                const port = independentQueueResume(queue.acquireOwner, scope => gateway.queue!.read(scope));
                if (guarded.getSnapshot().phase === "resumePending" || (await port.read(id)).paused) await guarded.continuePaused(port, () => alive.current && currentId.current === id && !archived);
                else await guarded.continueAdmitted(conversationId => port.read(conversationId));
            }
            else await guarded.check();
            if (!alive.current) return;
            if (guarded.getSnapshot().phase === "started") await finishGuarded();
            else notifyGuarded();
        } catch (cause) {
            if (alive.current) {const safe = diagnosticError(cause); notify.error(`无法读取队列，原请求已保留。 [code=${safe.code ?? "CONTEXT_QUEUE_UNAVAILABLE"}；C=${id}]`);}
        } finally {if (alive.current) {operation.current = false; setBusy(false);}}
    }
    useEffect(() => {if (guarded && guarded.getSnapshot().phase !== "idle" && !busy) notifyGuarded();}, [guarded, busy]);
    async function prepare(text: string, options: WritingAgentSubmitOptions,traceId?:string, external = false, queued?: AgentQueueAutoPrepareInput, onIgnored?: () => void, submissionRequestId?: string): Promise<void> {
        if (archived || handoffBlocked || taskObservation.state || operation.current || running || (!pending&&history.status !== "ready") || !nativeContext.ready) {
            onIgnored?.(); return;
        }
        if (recoveryBlocked) { onIgnored?.(); setRecoveryAttempt(value => value + 1); return; }
        if (guardedBlocked) {onIgnored?.(); void inspectGuarded(); return;}
        if (preview) {
            if (text === preview.input.text) await start();
            else {onIgnored?.(); notifyRecovery("上次发送结果尚未确认，当前修改已保留。请先核对原请求。", preview.requestId);}
            return;
        }
        if(options.includeConceptRetrieval&&!traceId){
            onIgnored?.();
            const previewRuntime = getAppRuntime();
            if (previewRuntime.nativeAgentPreview?.bootstrap.supported.includes(NATIVE_PREVIEW_OWNER_CAPABILITY)) {
                submissionRequestId = crypto.randomUUID();
                operation.current = true; setBusy(true);
                try {await freezeNewPreviewOwnerIntent(previewRuntime.nativeAgentPreview, storage, id, submissionRequestId);}
                catch (cause) {notify.error(`无法保存预览请求归属，草稿已保留。（${cause instanceof Error ? cause.message : "agent_preview_owner_intent_failed"}）`); return;}
                finally {if (alive.current) {operation.current = false; setBusy(false);}}
                if (!alive.current || currentId.current !== id) return;
            }
            await retrieval.search(text,options,submissionRequestId);return;
        }
        const preparationDraft = inputDraft.current;
        failureDraft.current = preparationDraft;
        const requestId = submissionRequestId ?? crypto.randomUUID();
        const pendingId = !external && !queued ? requestId : null;
        if (pendingId) setPendingSubmission({id:pendingId,conversationId:id,text,userInput:structuredClone(options.userInput ?? [])});
        const forgetPending = () => {if (pendingId) setPendingSubmission(value => value?.id === pendingId ? null : value);};
        const setPendingTurn = (turnId:string) => {if (pendingId) setPendingSubmission(value => value?.id === pendingId ? {...value,turnId} : value);};
        operation.current = true;
        queuedDispatchInput.current = queued?.inputId ?? null;
        const preflight = pendingId ? {requestId, canceled:false} : null;
        if (preflight) {sendPreflight.current = preflight; setPreflightSending(true);}
        setBusy(true);
        setError(undefined);
        try {
            if (!submissionRequestId) await freezeNewPreviewOwnerIntent(getAppRuntime().nativeAgentPreview, storage, id, requestId);
            if (preflight?.canceled) {if (alive.current) forgetPending(); return;}
            let capturedDraft: {text: string | null; input: string | null; version: string} | undefined;
            if (guarded) {
                const version = crypto.randomUUID(); storage.setItem(draftVersionKey, version);
                capturedDraft = {text: storage.getItem(draftKey), input: storage.getItem(inputKey), version};
            }
            await ensureCreated();
            if (preflight?.canceled) {if (alive.current) forgetPending(); return;}
            if (!alive.current) {if (external) messageRequest?.reject(new Error("聊天窗口已关闭，消息未发送。")); return;}
            const selections = external ? freezeAgentContext(recommendations.effective) : freezeAgentContext(recommendations.capture(requestId));
            const model = options.modelSelection ?? chatModel;
            await nativeContext.save(context);
            if (preflight?.canceled) {if (alive.current) forgetPending(); return;}
            if (!alive.current) {if (external) messageRequest?.reject(new Error("聊天窗口已关闭，消息未发送。")); return;}
            const request = { conversationId: id, requestId, text, resources: [...new Map([...selections.map(s=>s.resource),...(external ? messageRequest?.resources ?? [] : []),...turnContextResources(options.userInput)].map(resource=>[agentContextKey(resource),resource])).values()], userInput:options.userInput??[], ...(traceId?{retrieval:{traceId,query:text}}:{}), ...(model ? {model} : {}), ...(queued && queueState ? {queue:{inputId:queued.inputId, expectedRevision:queueState.revision}} : {}) };
            if (preflight && sendPreflight.current === preflight) {
                // Withdraw the action before starting a request that this
                // preflight-only cancellation cannot safely cancel.
                sendPreflight.current = null; flushSync(() => setPreflightSending(false));
            }
            if (guarded) {
                await guarded.begin(request, external || !!queued, capturedDraft!);
                if (!alive.current) {if (external) messageRequest?.reject(new Error("聊天窗口已关闭，请重新打开并核对原请求。")); return;}
                if (guarded.getSnapshot().phase === "idle" && guarded.getSnapshot().cause) {
                    forgetPending();
                    if (external) messageRequest?.reject(new Error(message("modelUnavailable")));
                    else if (queued) setError(message("modelUnavailable"));
                    else recoverAgentPreparationFailure({text, userInput: options.userInput ?? [], originalDraft: preparationDraft, read: () => inputDraft.current,
                        restore: value => {storage.setItem(draftKey, value.text); storage.setItem(inputKey, JSON.stringify(agentDraftInput(value))); inputDraft.current = value; setDraft(value.text);}, detail: message("modelUnavailable")});
                    return;
                }
                const preparedTurn = guarded.getSnapshot().record?.committed?.result.turnId;
                if (preparedTurn) setPendingTurn(preparedTurn);
                if (guarded.getSnapshot().phase === "ready") await guarded.start();
                const acceptedTurn = guarded.getSnapshot().record?.start?.result?.turnId;
                if (acceptedTurn) setPendingTurn(acceptedTurn);
                if (guarded.getSnapshot().phase === "started") {await finishGuarded(); if (external) messageRequest?.resolve();}
                else {notifyGuarded(); if (external) messageRequest?.reject(new Error("消息状态尚未确认，请在聊天窗核对原请求。"));}
                return;
            }
            const frozen = await gateway.prepare(request);
            setPendingTurn(frozen.turnId);
            preserveDraft.current = external || !!queued;
            // Preserve the frozen request for recovery before starting it.
            storage.setItem(pendingKey, JSON.stringify({ conversationId: id, requestId: frozen.requestId, contextSnapshotId: frozen.contextSnapshotId, phase: "prepared", preserveDraft: preserveDraft.current }));
            if (alive.current) {
                setPreview(frozen);
                setUncertain(false);
                const admitted = await startFrozen(frozen, false, { forwardRejection: external });
                if (external) { if (admitted) messageRequest?.resolve(); else messageRequest?.reject(new Error("消息未发送，请在聊天窗核对模型配置后重试。")); }
            }
        }
        catch (reason) {
            if (preflight?.canceled) {if (alive.current) forgetPending(); return;}
            // Once the durable preparation owns this request, a later local
            // failure cannot prove that it was never accepted. Keep its bubble
            // and its original recovery identity instead of reporting a retry.
            const guardedOwner = guarded?.getSnapshot().record?.intent.requestId === requestId;
            if (!guardedOwner) forgetPending();
            // Reader actions own their transient notification. Keep this error
            // out of the chat's status area so the same rejection appears once.
            if (alive.current && !external) {
                if (guardedOwner) notifyGuarded();
                else if (queued) setError(message(reason));
                else recoverAgentPreparationFailure({text, userInput: options.userInput ?? [], originalDraft: preparationDraft, read: () => inputDraft.current,
                    restore: value => {
                        storage.setItem(draftKey, value.text);
                        storage.setItem(inputKey, JSON.stringify(agentDraftInput(value)));
                        inputDraft.current = value;
                        setDraft(value.text);
                    }, detail: message(reason)});
            }
            if (external) messageRequest?.reject(new Error(message(reason)));
        }
        finally {
            if (alive.current) {
                if (preflight && sendPreflight.current === preflight) {sendPreflight.current = null; setPreflightSending(false);}
                if (preflight) setCancelSendingPending(false);
                setBusy(false);
                operation.current = false;
                if (queuedDispatchInput.current === queued?.inputId) queuedDispatchInput.current = null;
                setQueueRefresh(value => value + 1);
            }
        }
    }
    async function start(): Promise<void> {
        if (archived || taskObservation.state || !preview || operation.current)
            return;
        operation.current = true;
        setBusy(true);
        setError(undefined);
        try {
            const frozen = preview.queue ? await gateway.restore(preview) : preview;
            if (frozen.contextSnapshotId !== preview.contextSnapshotId) throw new Error("confirmationMismatch");
            await startFrozen(frozen, uncertain, {resumeQueue:!!frozen.queue});
        } catch (reason) {
            setError(message(reason));
        } finally {
            if (alive.current) {
                setBusy(false);
                operation.current = false;
            }
        }
    }
    async function startFrozen(frozen: AgentContextPreview, wasUncertain: boolean, feedback: { readonly forwardRejection?: boolean; readonly resumeQueue?: boolean } = {}): Promise<boolean> {
        setUncertain(true);
        try {
            storage.setItem(pendingKey, JSON.stringify({ conversationId: id, requestId: frozen.requestId, contextSnapshotId: frozen.contextSnapshotId, phase: "starting", preserveDraft: preserveDraft.current }));
            const receipt = feedback.resumeQueue ? await gateway.start(frozen, {resumeQueue:true}) : await gateway.start(frozen);
            if (!(await taskObservation.track(receipt))) return true;
            setQueueRefresh(value => value + 1);
            // The real and preview transports only admit the turn. Naming belongs
            // to the shared client and must never turn an accepted send into a retry.
            if (title === "新对话" && !(projection?.turnOrder.some(turn => turn !== frozen.turnId))) {
                void nameInitialConversation(id, frozen.requestId, frozen.input.text, { catalog: agentCatalogGateway, metadata: agentMetadataGateway })
                    .then(row => {
                        if (!row) return;
                        window.dispatchEvent(new Event(CONVERSATION_TITLE_CHANGED_EVENT));
                        if (alive.current && currentId.current === row.id) {
                            if (currentTitle.current === "新对话" || currentTitle.current === row.title) onRenamed?.(row);
                            onRefreshTitle?.();
                        }
                    })
                    .catch(() => { if (alive.current) onRefreshTitle?.(); });
            }
            // Acceptance, including exact retry, consumes only this conversation's draft.
            const next = preserveDraft.current && !frozen.queue ? context : { ...recommendations.commit(frozen.requestId), temporary: [] };
            if (frozen.queue) writeAgentContextState(id, next);
            if (!preserveDraft.current || frozen.queue) void nativeContext.save(next).catch(() => notify.error("消息已发送，但持续上下文保存失败，请重新核对。"));
            const consumeDraft = !preserveDraft.current && inputDraft.current === failureDraft.current
                && serializeAgentSourceReferences(inputDraft.current).text === frozen.input.text;
            if (!preserveDraft.current) {
                writeAgentContextState(id, next);
            }
            if (consumeDraft) {
                storage.removeItem(draftKey);
                storage.removeItem(inputKey); releaseRemovedEvidence(inputDraft.current.evidence); inputDraft.current={text:"",bindings:[]};
            }
            storage.removeItem(pendingKey);
            if (alive.current) {
                setContext(next);
                if (consumeDraft) setDraft("");
                setPreview(null);
                setUncertain(false);
                if (receipt.outcome === null)
                    setAwaitingTurn(receipt.turnId);
                history.reconnect();
                onStarted();
            }
            return true;
        }
        catch (reason) {
            if (alive.current) {
                // These native errors occur before runner admission. A previous
                // uncertain attempt cannot be ruled out by today's rejection.
                const rejected = !wasUncertain && isAgentStartRejection(reason);
                if (rejected) {
                    setPendingSubmission(value => value?.turnId === frozen.turnId && value.conversationId === id ? null : value);
                    setUncertain(false);
                    setPreview(null);
                    storage.removeItem(pendingKey);
                }
                if (rejected && feedback.forwardRejection) throw reason;
                if (rejected) recoverAgentPreparationFailure({text: frozen.input.text, userInput: frozen.input.userInput ?? [],
                    originalDraft: failureDraft.current, read: () => inputDraft.current,
                    restore: value => {inputDraft.current = value;storage.setItem(draftKey,value.text);storage.setItem(inputKey,JSON.stringify(agentDraftInput(value)));setDraft(value.text);},
                    failureMessage: AGENT_START_FAILURE, detail: message(reason)});
                else notifyRecovery("发送结果尚未确认，消息已保留在输入框中。重新发送将核对原请求，避免重复执行。", frozen.requestId);
                return !rejected;
            }
            return true;
        }
    }
    retryRef.current = start;
    useEffect(() => {
        if (preview && !busy && uncertain && projection?.turns.has(preview.turnId) && reconciledAdmission.current !== preview.requestId) {
            reconciledAdmission.current = preview.requestId;
            void retryRef.current(); return;
        }
        if (preview && !busy && lastRecoveryNotice.current !== preview.requestId)
            notifyRecovery(uncertain ? "上次发送结果尚未确认，请重新发送以核对原请求。" : "消息尚未发送，已保留，请重新发送。", preview.requestId);
    }, [preview,busy,uncertain,projection]);
    useEffect(() => {
        if (!messageRequest || externalRequest.current === messageRequest) return;
        if (messageRequest.options?.mode === "draft") {
            externalRequest.current = messageRequest;
            if (archived || recoveryBlocked || guardedBlocked || taskObservation.blocked || preview || operation.current) { messageRequest.reject(new Error("请先恢复待发送请求或新建对话。")); return; }
            try {
                const previous = inputDraft.current.evidence ?? [], incoming = messageRequest.options.evidence ?? [];
                const evidence = [...previous, ...incoming];
                const next = {...inputDraft.current, text: inputDraft.current.text || messageRequest.text, evidence};
                // Validate and persist the whole draft before acknowledging capture.
                const input = agentDraftInput(next);
                const validated = agentDraftFromInput(next.text, input);
                const persistentKeys = new Set(context.persistent.map(entry => agentContextKey(entry.resource)));
                const contextNext = {...context, temporary: [...new Map([...context.temporary, ...(messageRequest.resources ?? [])
                    .filter(resource => !persistentKeys.has(agentContextKey(resource)))
                    .map(resource => ({resource, label: messageRequest.options?.evidence?.[0]?.title ?? "选区来源"}))].map(entry => [agentContextKey(entry.resource), entry])).values()]};
                storage.setItem(draftVersionKey, crypto.randomUUID()); storage.setItem(inputKey, JSON.stringify(input)); storage.setItem(draftKey, next.text);
                writeAgentContextState(id, contextNext);
                inputDraft.current = validated; setDraft(next.text); setContext(contextNext);
                messageRequest.resolve();
            } catch (reason) { messageRequest.reject(reason); }
            return;
        }
        if (archived || recoveryBlocked || guardedBlocked || handoffBlocked || taskObservation.state || preview || running || retrieval.pending || operation.current) {
            externalRequest.current = messageRequest;
            messageRequest.reject(new Error(archived ? "此对话已归档，请恢复或新建对话后重试。" : "聊天窗正在处理请求，请完成或恢复当前请求后重试。"));
            return;
        }
        if (!pending && history.status !== "ready") {
            if (history.status === "error") { externalRequest.current = messageRequest; messageRequest.reject(new Error("无法读取聊天历史，请重试。")); }
            return;
        }
        externalRequest.current = messageRequest;
        void prepare(messageRequest.text, { contextScope: "manuscript", ...(messageRequest.options?.evidence?.length ? {userInput: agentDraftInput({text: messageRequest.text, bindings: [], evidence: messageRequest.options.evidence})} : {}), ...(chatModel ? { modelSelection: chatModel } : {}) }, undefined, true);
    }, [messageRequest, archived, recoveryBlocked, guardedBlocked, handoffBlocked, taskObservation.state, preview, running, retrieval.pending, pending, history.status]);
    const taskSettling = !!taskObservation.state && !running;
    const guardedSnapshot = guarded?.getSnapshot();
    const recoveredPendingTurn = pendingSubmission && guardedSnapshot?.record?.intent.requestId === pendingSubmission.id
        ? guardedSnapshot.record.committed?.result.turnId : undefined;
    useEffect(() => {
        if (recoveredPendingTurn) setPendingSubmission(value => value && value.turnId !== recoveredPendingTurn ? {...value,turnId:recoveredPendingTurn} : value);
    }, [recoveredPendingTurn]);
    const identityFailure = diagnosticError(guardedSnapshot?.cause);
    const taskIdentityUnavailable = identityFailure.code === "AGENT_TASK_IDENTITY_UNAVAILABLE"
        || (guardedSnapshot?.phase === "unknown" && !!guardedSnapshot.record?.start?.result && !guardedSnapshot.record.start.result.runId);
    const taskDetail = taskObservation.state?.diagnostic, taskIdentity = taskObservation.state?.identity, observationFailure = taskObservation.state?.observationFailure;
    const taskFields = [`code=${taskDetail?.code ?? "AGENT_TASK_OBSERVATION_UNCONFIRMED"}`, `C=${id}`,
        ...(taskIdentity ? [`T=${taskIdentity.turnId}`, `run=${taskIdentity.runId}`] : []),
        ...(typeof taskDetail?.requestId === "string" ? [`R=${taskDetail.requestId}`] : []),
        ...(typeof taskDetail?.operationId === "string" ? [`operation=${taskDetail.operationId}`] : []),
        ...(taskDetail?.phase ? [`phase=${taskDetail.phase}`] : []), ...(taskDetail?.safeCause ? [`cause=${taskDetail.safeCause}`] : []),
        ...(observationFailure ? [`observationPhase=${observationFailure.phase}`, `observationCause=${observationFailure.cause}`, ...(observationFailure.errorCode ? [`observationCode=${observationFailure.errorCode}`] : []), ...(observationFailure.errorName ? [`observationError=${observationFailure.errorName}`] : [])] : []),
        ...(typeof taskDetail?.sqliteCode === "string" ? [`sqliteCode=${taskDetail.sqliteCode}`] : []),
        ...(typeof taskDetail?.sqliteExtendedCode === "number" ? [`sqlite=${taskDetail.sqliteExtendedCode}`] : [])];
    const messages = useMemo(() => projectConversationMessages(projection), [projection]);
    useEffect(() => {
        if (pendingSubmission && hasConfirmedSubmissionMessage(pendingSubmission, messages))
            setPendingSubmission(value => value?.id === pendingSubmission.id ? null : value);
    }, [messages, pendingSubmission]);
    const taskIssue = taskObservation.state?.status === "unconfirmed" || taskObservation.state?.status === "recovery";
    const taskInTurn = !!taskIdentity && messages.some(message => message.turnId === taskIdentity.turnId);
    const taskFeedback = taskIssue ? <AgentRecoveryMessage id="task-observation" label="运行状态恢复" content="无法确认运行状态，已暂停新发送和队列自动启动；历史与草稿保留。" diagnosticText={taskFields.join("；")}>
      <div className="agent-failure-actions">{taskIdentity ? <button type="button" onClick={taskObservation.recheck}>核对原运行</button>
        : guarded?.getSnapshot().record?.start && <button type="button" disabled={busy} onClick={() => void inspectGuarded()}>核对原请求</button>}</div></AgentRecoveryMessage> : null;
    const hiddenHandoffInputs=useHiddenHandoffQueueInputs(id,id,handoff.state,queue.state,messages);
    const handoffConflict = handoff.state?.phase === "waiting" && handoff.state.items.some(item =>
      item.history === "incompleteOrConflicting" && projection?.turns.get(item.intent.turnId)?.outcome != null);
    const handoffError = handoffInputBlocked && (!handoff.available || handoff.state?.phase === "uncertain" || handoff.state?.phase === "error" || handoffConflict);
    const handoffIntents = [...(handoff.state?.items.map(item => item.intent) ?? []), ...(handoff.state?.intent ? [handoff.state.intent] : [])];
    const controls = <><AgentPendingSubmissionBubble pending={pendingSubmission} messages={messages} resourceGateways={resources}/><AgentPendingSteeringBubbles messages={messages} conversationId={id} direct={steering.state.awaitingHistory} handoffs={handoffIntents} resourceGateways={resources}/>
    {(retrieval.controls || archived || nativeContext.recovery || taskObservation.blocked || taskIdentityUnavailable || steering.unresolved || handoffError || !!steering.state.transferFailures?.length) ? <AgentSubmissionFeedback label="对话状态" className="agent-context-status">
    {retrieval.controls}
    {nativeContext.recovery && <ContextConfigurationRecovery conversationId={id} recovery={nativeContext.recovery} detail={nativeContext.detail}
      conflicts={nativeContext.conflicts} onRetry={() => void nativeContext.retry().then(() => setError(undefined)).catch(reason => setError(`无法核对上下文：${message(reason)}`))}
      onRepair={() => void nativeContext.repair().then(() => setError(undefined)).catch(reason => setError(`无法修复上下文：${message(reason)}`))}
      onResolve={(conflict,action) => void nativeContext.resolve(conflict,action).then(() => setError(undefined)).catch(reason => setError(`无法处理上下文冲突：${message(reason)}`))}/>}
    <AgentSteeringReceipt hidePendingMessages awaitingHistory={steering.state.awaitingHistory} transferFailures={steering.state.transferFailures} onRetryTransfer={inputId=>void reconcileSteeringTransfer(queue.controller.current,inputId,steering.retryConfirmation)} uncertain={steering.uncertain} pending={steering.pending} text={steering.state.request?.text} status={steering.status} {...(steering.state.request?.inputId?{requestId:steering.state.request.inputId}:{})} disabled={history.status!=="ready"||archived} onRetry={()=>void steering.retry()}/>
    {handoffError && <AgentSteeringHandoffRecovery available={handoff.available} state={handoff.state} disabled={history.status!=="ready"||archived} canRetry={!!handoff.state?.intent && projection?.activeTurn===handoff.state.intent.turnId} queueContainsInput={!!queueState?.inputs.some(input=>input.inputId===handoff.state?.intent?.queueInputId || handoff.state?.items.some(item=>item.intent.queueInputId===input.inputId))} onCheck={()=>void handoff.refresh()} onRetry={()=>void handoff.retry()}/>}
    {archived && <p role="status">此对话已归档。历史和草稿保留，恢复后可继续。</p>}
    {!taskInTurn && taskFeedback}
    {taskIdentityUnavailable && <AgentRecoveryMessage id="task-identity" label="运行状态恢复" content="原请求已启动，但暂时无法观察运行状态；已暂停新发送，草稿和原请求已保留。" diagnosticText={`code=AGENT_TASK_IDENTITY_UNAVAILABLE；C=${id}；R=${guardedSnapshot?.record?.intent.requestId}；T=${guardedSnapshot?.record?.start?.result?.turnId}；phase=start_identity；cause=${typeof identityFailure.cause === "string" ? identityFailure.cause : "unclassified"}${typeof identityFailure.code === "string" ? `；sourceCode=${identityFailure.code}` : ""}`}>
      <div className="agent-failure-actions"><button type="button" disabled={busy} onClick={() => void inspectGuarded()}>核对原请求</button></div></AgentRecoveryMessage>}
  </AgentSubmissionFeedback> : null}</>;
    const queuePreparationStatus = independentQueuePreparingPresentation(queueState?.preparingInputId,guardedSnapshot,busy,queuedDispatchInput.current);
    function cancelPreflightSending(): void {
        const current = sendPreflight.current;
        if (!current || current.canceled) return;
        current.canceled = true; setCancelSendingPending(true);
    }
    const queueControlsDisabled = archived || (!pending && !nativeContext.ready) || busy || recoveryBlocked || guardedBlocked || taskObservation.blocked || taskSettling || !!preview || steering.unresolved || handoffBlocked || !!steering.state.awaitingHistory?.length;
    const recoverQueue = () => {
        if (!queueState?.preparingInputId || queue.controller.current?.getSnapshot().queue?.preparingInputId !== queueState.preparingInputId) return;
        setError(undefined);setQueueRefresh(value=>value+1);
    };
    const queueControls = queue.state ? <WritingAgentTaskStack queue={{
        state:queue.state, hiddenInputIds:hiddenHandoffInputs, preparingStatus:queuePreparationStatus, disabled:queueControlsDisabled,
        ...(running && steeringScope ? {steerDisabled:archived || !nativeContext.ready || busy || stopping || recoveryBlocked || guardedBlocked || taskObservation.blocked || taskSettling || !!preview || steering.unresolved || handoffInputBlocked || !handoff.available || !handoff.canSubmit(steeringScope.turnId)} : {}),
        onEdit: async edit => {setError(undefined); return queue.controller.current?.edit(edit) ?? false;},
        onReload: () => {setError(undefined);autoQueue.markQueued();return queue.controller.current?.reload();},
        onRetry: () => {setError(undefined);autoQueue.markQueued();void queue.controller.current?.retry();},
        ...(running && steeringScope && gateway.steer && handoff.available && !queueBlocked && nativeContext.ready ? {onSteer: async (inputId:string) => {
            const snapshot=queue.controller.current?.getSnapshot();if(!snapshot?.queue || snapshot.loading || snapshot.editing || snapshot.pending || snapshot.uncertain || snapshot.error || handoffInputBlocked)return false;
            return handoff.submit(snapshot.queue,steeringScope.turnId,inputId);
        }} : !running ? {onSteer: async (inputId: string) => {
            const controller = queue.controller.current;
            const state = controller?.getSnapshot().queue;
            if (!controller || !state) return false;
            if (state.inputs[0]?.inputId !== inputId && !await controller.edit({type:"reorder",inputIds:[inputId,...state.inputs.filter(input=>input.inputId!==inputId).map(input=>input.inputId)]})) return false;
            setError(undefined);autoQueue.markQueued();
            return controller.edit({type:"resumeInput",inputId});
        },steerStartsTurn:true} : {}),
    }}/> : undefined;
    return <WritingAgentPanel approvalConversationId={id} turnFeedback={turnId => taskInTurn && turnId === taskIdentity?.turnId ? taskFeedback : null} isSubmitting={busy} composerAccessory={queueControls} {...(preflightSending ? {onCancelSending:cancelPreflightSending, cancelSendingPending} : {})} {...(queuePreparationStatus === "sending" || queuePreparationStatus === "recovery" ? {queuePreparation: {status:queuePreparationStatus, onRecover:recoverQueue, recoveryDisabled:queueControlsDisabled || !!queue.state?.loading || !!queue.state?.editing || !!queue.state?.uncertain || hiddenHandoffInputs.length > 0}} : {})} {...(gateway.queue ? {onQueue:enqueue,queueDisabled:archived || (!pending && !nativeContext.ready) || busy || stopping || recoveryBlocked || guardedBlocked || taskObservation.blocked || !!preview || !!queueBlocked || handoffInputBlocked || steering.unresolved} : {})} {...(gateway.steer && steeringScope && nativeContext.ready && !queueBlocked && !handoffInputBlocked && !stopping && !archived && !busy && !recoveryBlocked && !guardedBlocked && !taskObservation.blocked && !preview ? {onSteer:(text:string,userInput?:readonly import("@research-assistant/core/agent").AgentUserInput[])=>{const accepted=registry?undefined:captureSteeringDraft(text);void steering.submit(text,{userInput:userInput??[],...(accepted?{accepted}:{}),queueRejected:(request,ack)=>recoverRejectedSteer(queue.acquireOwner(),request,ack??accepted??(()=>undefined))});},steeringPending:steering.pending} : {})} {...(contextUsage ? {contextUsage} : {})} onModelSelectionChange={setChatModel} resourceGateways={resources} draftResources={{draft:()=>inputDraft.current,set:next=>{const previous=inputDraft.current;storage.setItem(draftVersionKey,crypto.randomUUID());storage.setItem(inputKey,JSON.stringify(agentDraftInput(next)));inputDraft.current=next;releaseRemovedEvidence(previous.evidence,next.evidence);}}} independentContext showLauncher={false} open={open} onOpenChange={onOpenChange} headerControls={headerControls} conversationTitle={title || "新对话"} {...(!pending ? {conversationRename: {conversationId: id, catalog: agentCatalogGateway, ...(onRenamed ? {onRenamed} : {}), ...(onRefreshTitle ? {onConflict: onRefreshTitle} : {})}} : {})} historyReady={pending||history.status === "ready"} historyError={history.status === "error"} messages={messages} hasPendingSubmission={!!pendingSubmission && !hasConfirmedSubmissionMessage(pendingSubmission,messages)} draft={draft} onDraftChange={changeDraft} inputDisabled={archived || busy || retrieval.pending} {...(handoffInputBlocked ? {disabledReason:handoffError ? "队列引导尚未核对" : "引导等待处理"} : steering.unresolved ? {disabledReason:steering.status} : taskObservation.state?.status === "checking" ? {disabledReason:"正在核对运行状态……"} : taskObservation.blocked || taskIdentityUnavailable ? {disabledReason:"原运行需要核对"} : taskSettling ? {disabledReason:"原运行正在收尾"} : {})} submittingText={recoveryBlocked ? "正在核对对话状态……" : "正在核对请求……"} isRunning={running} {...(activeTurn && projection?.turns.get(activeTurn)?.startedAtMs !== undefined ? { thinkingStartedAtMs: projection.turns.get(activeTurn)!.startedAtMs } : {})} onSubmit={(text, options) => {let ignored = false; void prepare(text, options, undefined, false, undefined, () => {ignored = true;}); return ignored ? false : undefined;}} onCancel={() => void stop()} contextControls={<AgentContextControl recommendations={recommendations} pending={pending} captureDirectoryNavigation={captureDirectoryNavigation} beforeDirectoryChange={ensureCreated} conversationId={id} {...(onDirectoryChanged?{onDirectoryChanged}:{})} state={context} onChange={changeContext} gateway={gateway} disabled={archived || (!pending && !nativeContext.ready) || busy || recoveryBlocked || guardedBlocked || preview !== null || running || retrieval.pending || nativeContext.loading || nativeContext.saving}/>} submissionControls={controls}/>;
}
