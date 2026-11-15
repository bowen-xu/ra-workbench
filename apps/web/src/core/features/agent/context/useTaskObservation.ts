import {useCallback, useEffect, useRef, useState} from "react";
import {decodeAgentTaskObservationRequest, decodeAgentTaskObservationResponse, type AgentStartResponse, type AgentTaskDiagnostic,
  type AgentTaskObservationRequest} from "@research-assistant/core/agent";
import type {AgentTaskObservationGateway} from "@research-assistant/data/agent";
import {diagnosticError} from "../../../services/diagnosticError";
import {flushStorage} from "../../../runtime/storagePersistence";

type Snapshot = {readonly identity: AgentTaskObservationRequest | null; readonly status: "tracking" | "checking" | "recovery" | "unconfirmed";
  readonly diagnostic?: AgentTaskDiagnostic | null; readonly observationFailure?: {readonly phase: string; readonly cause: string; readonly errorCode?: string; readonly errorName?: string}};

function failure(phase: string, reason?: unknown) {
  const detail = reason === undefined ? undefined : diagnosticError(reason);
  const errorCode = typeof detail?.code === "string" ? detail.code
    : typeof reason === "string" && /^[A-Z][A-Z0-9_]{2,100}$/u.test(reason) ? reason : undefined;
  const cause = detail?.code === "AGENT_TASK_OBSERVATION_IDENTITY_MISMATCH" ? "identity_mismatch"
    : reason instanceof Error && reason.name === "AgentTaskObservationDecodeError" ? "invalid_response"
    : reason instanceof Error && reason.message === "task observation identity mismatch" ? "identity_mismatch"
    : typeof detail?.cause === "string" ? detail.cause : "unclassified";
  return {phase, cause,
    ...(typeof detail?.name === "string" ? {errorName: detail.name} : {}),
    ...(errorCode ? {errorCode} : {})};
}

function restore(storage: Storage, key: string, conversationId: string): Snapshot | null {
  try {
    const raw = storage.getItem(key);
    if (raw === null) return null;
    const identity = decodeAgentTaskObservationRequest(JSON.parse(raw));
    if (identity.conversationId !== conversationId.toLowerCase()) throw new Error("identity mismatch");
    return {identity, status: "checking"};
  } catch (error) { return {identity: null, status: "unconfirmed", observationFailure: failure("restore_locator", error)}; }
}
/** A locator only: it never authorizes a start, retry, or durable recovery. */
export function useTaskObservation(conversationId: string, storage: Storage, gateway?: AgentTaskObservationGateway) {
  const key = `research-assistant:agent-task-observation:v1:${conversationId}`;
  const [state, setState] = useState<Snapshot | null>(() => restore(storage, key, conversationId));
  const [attempt, setAttempt] = useState(0);
  const current = useRef(state); current.current = state;
  const scope = useRef(conversationId); scope.current = conversationId;
  const restoredScope = useRef({storage, key});
  useEffect(() => {
    if (restoredScope.current.storage === storage && restoredScope.current.key === key) return;
    restoredScope.current = {storage, key};
    setState(restore(storage, key, conversationId));
  }, [storage, key, conversationId]);
  const track = useCallback(async (receipt: AgentStartResponse) => {
    if (!receipt.runId) return true;
    let identity: AgentTaskObservationRequest;
    try {
      identity = decodeAgentTaskObservationRequest({conversationId: receipt.conversationId, turnId: receipt.turnId, runId: receipt.runId});
      if (identity.conversationId !== conversationId.toLowerCase()) throw new Error("identity mismatch");
    } catch (error) {if (scope.current === conversationId) setState({identity: null, status: "unconfirmed", observationFailure: failure("decode_start_identity", error)}); return false;}
    try {
      const existing = storage.getItem(key), encoded = JSON.stringify(identity);
      if (existing !== null && existing !== encoded) {
        if (scope.current === conversationId) setState({identity: null, status: "unconfirmed", observationFailure: {phase: "save_locator", cause: "locator_conflict"}}); return false;
      }
      storage.setItem(key, encoded); await flushStorage(storage);
      if (storage.getItem(key) !== encoded) {
        if (scope.current === conversationId) setState({identity: null, status: "unconfirmed", observationFailure: {phase: "save_locator", cause: "locator_changed"}});
        return false;
      }
    } catch (error) {
      if (scope.current === conversationId) setState({identity, status: "unconfirmed", observationFailure: failure("save_locator", error)}); return false;
    }
    if (scope.current === conversationId) setState(previous =>
      previous?.identity?.runId === identity.runId && previous.identity.turnId === identity.turnId
        && previous.status !== "tracking" ? previous : {identity, status: "tracking"});
    return true;
  }, [conversationId, key, storage]);
  useEffect(() => {
    if (!state || (state.status !== "tracking" && state.status !== "checking") || !state.identity) return;
    const identity = state.identity;
    if (!gateway) {setState({identity, status: "unconfirmed", observationFailure: {phase: "observe", cause: "gateway_missing"}}); return;}
    let live = true, timer: ReturnType<typeof setTimeout> | undefined;
    const same = () => live && current.current?.identity?.runId === identity.runId
      && current.current.identity.turnId === identity.turnId && current.current.identity.conversationId === identity.conversationId;
    const poll = async () => {
      try {
        const result = decodeAgentTaskObservationResponse(await gateway.observe(identity));
        if (!same()) return;
        if (result.conversationId !== identity.conversationId || result.turnId !== identity.turnId || result.runId !== identity.runId)
          throw new Error("task observation identity mismatch");
        if (result.status.type === "pending") {
          if (current.current?.status === "checking") setState({identity, status: "tracking"});
          else timer = setTimeout(() => void poll(), 1000);
          return;
        }
        if (result.status.type === "unavailable") {setState({identity, status: "unconfirmed", observationFailure: {phase: "observe", cause: "owner_unavailable"}}); return;}
        if (result.status.type === "finished" && (result.status.completion === "recoveryRequired" || result.status.diagnostic !== null)) {
          setState({identity, status: "recovery", diagnostic: result.status.diagnostic}); return;
        }
        try {
          if (storage.getItem(key) !== JSON.stringify(identity)) {setState({identity, status: "unconfirmed", observationFailure: {phase: "clear_locator", cause: "locator_changed"}}); return;}
          storage.removeItem(key);
          await flushStorage(storage);
          if (!same()) return;
          setState(null);
        } catch (error) {
          // If deleting the locator could not be synchronized, keep the local
          // view fenced as well. Never overwrite a different locator.
          try {
            if (storage.getItem(key) === null) {storage.setItem(key, JSON.stringify(identity)); await flushStorage(storage);}
          } catch { /* The unsynchronized result stays unconfirmed. */ }
          if (same()) setState({identity, status: "unconfirmed", observationFailure: failure("clear_locator", error)});
        }
      } catch (error) {if (same()) setState({identity, status: "unconfirmed", observationFailure: failure("observe", error)});}
    };
    void poll();
    return () => {live = false; if (timer !== undefined) clearTimeout(timer);};
  }, [state, gateway, storage, key, attempt]);
  const recheck = useCallback(() => {
    setState(previous => previous?.identity ? {...previous, status: "checking"} : previous);
    setAttempt(value => value + 1);
  }, []);
  const clearSettled = useCallback(async (receipt: AgentStartResponse) => {
    let raw: string | null = null, identity: AgentTaskObservationRequest | null = null;
    try {
      const currentIdentity = current.current?.identity;
      if (currentIdentity && (currentIdentity.conversationId !== receipt.conversationId
        || currentIdentity.turnId !== receipt.turnId || (receipt.runId && currentIdentity.runId !== receipt.runId))) return false;
      raw = storage.getItem(key);
      if (raw !== null) {
        identity = decodeAgentTaskObservationRequest(JSON.parse(raw));
        if (identity.conversationId !== receipt.conversationId || identity.turnId !== receipt.turnId
          || (receipt.runId && identity.runId !== receipt.runId)) return false;
        storage.removeItem(key);
        await flushStorage(storage);
        if (storage.getItem(key) !== null) return false;
      }
      if (scope.current === conversationId) setState(null);
      return true;
    } catch (error) {
      try {if (raw !== null && storage.getItem(key) === null) {storage.setItem(key, raw); await flushStorage(storage);}} catch { /* Keep the fence. */ }
      if (scope.current === conversationId) setState({identity,status:"unconfirmed",observationFailure:failure("clear_settled_locator", error)});
      return false;
    }
  }, [conversationId, key, storage]);
  return {state, track, recheck, clearSettled, blocked: state?.status === "recovery" || state?.status === "unconfirmed" || state?.status === "checking"};
}
