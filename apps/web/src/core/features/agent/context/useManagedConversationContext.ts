import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { decodeAgentSqlImmediateFailure, type AgentContextState } from "@research-assistant/core/agent";
import type { DocumentConflict } from "@research-assistant/core/hbs";
import type { AgentContextGateway } from "../../../services/agentContextGateway";
import { getAppStorage } from "../../../runtime/appStorage";
import { contextStorageKey, writeConversationContext } from "./contextStorage";
import { contextConfigurationStartup } from "./contextConfigurationStartupFactory";
import type { ContextConfigurationOwner } from "./contextConfigurationOwner";
import type { ContextConfigurationStartup } from "./contextConfigurationStartup";

type Recovery = "seed" | "save" | "local" | "conflict" | "error" | null;
type Mode = "pending" | "readonly" | "active";
interface State { readonly ready: boolean; readonly loading: boolean; readonly saving: boolean; readonly recovery: Recovery;
  readonly conflicts: readonly DocumentConflict[]; readonly detail: string | undefined }
interface Session {
  readonly id: string; readonly storage: Storage; readonly gateway: AgentContextGateway;
  startup?: ContextConfigurationStartup; lease?: { readonly owner: ContextConfigurationOwner; readonly release: () => void }; mode: Mode;
  opening?: Promise<ContextConfigurationOwner>; unsubscribe?: () => void; mounted: boolean; readOnly: boolean; tasks: number; saves: number;
  seedPending: boolean; subscriptionPending: boolean; subscriptionFailed: boolean; subscriptionFailure?: unknown; reconnect?: () => Promise<void>;
  refreshes: number; refreshFailed: boolean; refreshFailure?: unknown; deferredRefresh: boolean; deferredGeneration: number;
}
const idle: State = { ready: false, loading: true, saving: false, recovery: null, conflicts: [], detail: undefined };
function detail(reason: unknown): string {
  const failure = decodeAgentSqlImmediateFailure(reason);
  if (failure) {
    const diagnostic = failure.diagnostic;
    return [`code=${failure.code}`, ...(diagnostic?.operationId ? [`operation=${diagnostic.operationId}`] : []),
      ...(diagnostic?.phase ? [`phase=${diagnostic.phase}`] : []), ...(diagnostic?.safeCause ? [`cause=${diagnostic.safeCause}`] : []),
      ...(diagnostic?.sqliteCode ? [`sqliteCode=${diagnostic.sqliteCode}`] : []),
      ...(diagnostic?.sqliteExtendedCode != null ? [`sqlite=${diagnostic.sqliteExtendedCode}`] : [])].join("；");
  }
  return reason instanceof Error ? reason.message : typeof reason === "string" ? reason : "CONTEXT_CONFIGURATION_UNAVAILABLE";
}

export function useManagedConversationContext(id: string | null, pending: boolean, gateway: AgentContextGateway,
  changed: (value: AgentContextState) => void, failed: (reason: unknown) => void, readOnly: boolean, enabled: boolean) {
  const storage = getAppStorage(), current = useRef<Session | null>(null), callbacks = useRef({ changed, failed });
  const mode: Mode = readOnly ? "readonly" : pending ? "pending" : "active";
  callbacks.current = { changed, failed };
  if (!id || !enabled) current.current = null;
  else if (!current.current || current.current.id !== id || current.current.gateway !== gateway || current.current.storage !== storage)
    current.current = { id, storage, gateway, mode, mounted: false, readOnly, tasks: 0, saves: 0, seedPending: false,
      subscriptionPending: !!gateway.subscribeConfiguration && !pending && !readOnly, subscriptionFailed: false, refreshes: 0, refreshFailed: false,
      deferredRefresh: false, deferredGeneration: 0 };
  const session = current.current;
  if (session) { session.readOnly = readOnly; session.mode = mode; }
  const [state, setState] = useState<{ readonly session: Session | null; readonly mode: Mode; readonly value: State }>({ session, mode, value: idle });
  const active = useCallback((owner: Session) => current.current === owner && owner.mounted, []);
  const publish = useCallback((host: Session) => {
    if (!active(host) || host.mode !== "active" || !host.lease) return;
    const snapshot = host.lease.owner.getSnapshot(), hbs = snapshot.session;
    try {
      const encoded = JSON.stringify(hbs.content);
      if (getAppStorage() === host.storage && host.storage.getItem(contextStorageKey(host.id)) !== encoded)
        writeConversationContext(host.id, hbs.content);
      callbacks.current.changed(hbs.content);
    } catch (reason) { callbacks.current.failed(reason); }
    const waiting = hbs.pending || !!hbs.queued;
    const recovery: Recovery = hbs.preservationError ? "local" : hbs.conflicts.length ? "conflict"
      : snapshot.originalPending || waiting && hbs.message.startsWith("MODEL_RESULT_UNKNOWN") ? "save" : host.subscriptionFailed || host.refreshFailed ? "error" : null;
    setState({ session: host, mode: host.mode, value: { ready: !recovery && !waiting && !host.saves && !host.subscriptionPending && !host.refreshes && !host.deferredRefresh,
      loading: !recovery && (host.subscriptionPending || !!host.refreshes || host.deferredRefresh), saving: !recovery && (waiting || !!host.saves), recovery,
      conflicts: hbs.conflicts, detail: hbs.preservationError ?? (recovery === "save" ? `${hbs.message}${snapshot.transportFailure ? ` ${detail(snapshot.transportFailure)}` : ""}` : recovery === "error" ? detail(host.subscriptionFailed ? host.subscriptionFailure : host.refreshFailure) : undefined) } });
  }, [active]);
  const release = useCallback((host: Session) => {
    if (host.mounted || host.tasks || !host.lease) return;
    host.unsubscribe?.(); delete host.unsubscribe; host.lease.release(); delete host.lease;
  }, []);
  const withOwner = useCallback(async <T,>(host: Session, operation: (owner: ContextConfigurationOwner) => Promise<T>): Promise<T> => {
    host.tasks++;
    try {
      if (!host.opening && !host.lease) {
        const save = host.gateway.configurationSave, seed = host.gateway.configurationSeed;
        if (!save || !seed) throw new Error("CONTEXT_CONFIGURATION_GATEWAY_UNAVAILABLE");
        host.startup ??= contextConfigurationStartup(host.storage, host.id, save, seed);
        const startup = host.startup;
        const opening = startup.open(() => !host.readOnly).then(result => {
          if (result.type === "seedPending") { host.seedPending = true; throw new Error("CONTEXT_CONFIGURATION_SEED_ORIGINAL_PENDING"); }
          host.seedPending = false; host.lease = result.lease;
          result.lease.owner.setWritable(!host.readOnly);
          host.unsubscribe = result.lease.owner.subscribe(() => publish(host));
          if (result.initializationError) callbacks.current.failed(result.initializationError);
          publish(host); return result.lease.owner;
        });
        host.opening = opening;
        void opening.then(() => { if (host.opening === opening) delete host.opening; }, () => { if (host.opening === opening) delete host.opening; });
      }
      const owner = host.lease?.owner ?? await host.opening;
      if (!owner) throw new Error("CONTEXT_CONFIGURATION_OWNER_UNAVAILABLE");
      return await operation(owner);
    } finally { host.tasks--; release(host); }
  }, [publish, release]);
  const report = useCallback((host: Session, reason: unknown) => {
    if (!active(host) || host.mode !== "active") return;
    callbacks.current.failed(reason);
    setState({ session: host, mode: host.mode, value: { ...idle, loading: false, recovery: host.seedPending ? "seed" : "error", detail: detail(reason) } });
  }, [active]);
  const catchUpDeferred = useCallback(async (host: Session, owner: ContextConfigurationOwner) => {
    if (!host.deferredRefresh) return;
    const snapshot = owner.getSnapshot();
    if (snapshot.originalPending || snapshot.session.pending || snapshot.session.queued || snapshot.session.preservationError || snapshot.session.conflicts.length) return;
    const generation = host.deferredGeneration;
    try {
      await owner.refresh();
      if (generation === host.deferredGeneration) host.deferredRefresh = false;
    } catch (reason) { host.refreshFailed = true; host.refreshFailure = reason; throw reason; }
  }, []);

  useLayoutEffect(() => {
    if (session?.lease) session.lease.owner.setWritable(!readOnly);
  }, [session, readOnly]);
  useEffect(() => {
    if (!session) return;
    session.mounted = true;
    if (pending || readOnly) { setState({ session, mode, value: { ...idle, ready: pending, loading: false } }); return () => { session.mounted = false; release(session); }; }
    setState({ session, mode, value: idle });
    void withOwner(session, async () => undefined).catch(reason => report(session, reason));
    return () => { session.mounted = false; release(session); };
  }, [session, pending, readOnly, mode, withOwner, report, release]);
  useEffect(() => {
    if (!session || pending || readOnly || !gateway.subscribeConfiguration) return;
    session.subscriptionPending = true; publish(session);
    let enabledSubscription = true, queue = Promise.resolve(), unlisten: (() => void) | undefined, connecting: Promise<void> | undefined;
    const refresh = (): Promise<void> => {
      session.refreshes++; publish(session);
      const task = queue.then(async () => {
        try {
          if (!enabledSubscription) return;
          await withOwner(session, async owner => {
            if (!enabledSubscription || session.readOnly) return;
            const snapshot = owner.getSnapshot();
            if (snapshot.originalPending || snapshot.session.pending || snapshot.session.queued || snapshot.session.preservationError || snapshot.session.conflicts.length) {
              session.deferredRefresh = true; session.deferredGeneration++; return;
            }
            const generation = session.deferredGeneration;
            await owner.refresh();
            if (generation === session.deferredGeneration) session.deferredRefresh = false;
          });
        } catch (reason) {
          if (enabledSubscription) {
            const snapshot = session.lease?.owner.getSnapshot();
            if (snapshot && (snapshot.originalPending || snapshot.session.pending || snapshot.session.preservationError || snapshot.session.conflicts.length)) {
              session.deferredRefresh = true; session.deferredGeneration++;
            } else {
              session.refreshFailed = true; session.refreshFailure = reason; report(session, reason);
            }
          }
          throw reason;
        } finally { session.refreshes--; publish(session); }
      });
      queue = task.catch(() => undefined);
      return task;
    };
    const connect = (catchUp: boolean): Promise<void> => {
      if (unlisten) return Promise.resolve();
      if (connecting) return connecting;
      const running = (async () => {
        try {
          const stop = await gateway.subscribeConfiguration!(session.id, () => { void refresh().catch(() => undefined); });
          if (!enabledSubscription) { stop(); return; }
          unlisten = stop;
          // A newly registered stream must catch changes since the initial read.
          if (catchUp) { await refresh(); if (!enabledSubscription) return; session.subscriptionPending = false; publish(session); }
        } catch (reason) {
          if (enabledSubscription) { session.subscriptionPending = false; session.subscriptionFailed = true; session.subscriptionFailure = reason; report(session, reason); }
          throw reason;
        }
      })();
      connecting = running;
      void running.then(() => { if (connecting === running) connecting = undefined; }, () => { if (connecting === running) connecting = undefined; });
      return running;
    };
    const reconnect = () => connect(false);
    session.reconnect = reconnect;
    void connect(true).catch(() => undefined);
    return () => { enabledSubscription = false; unlisten?.(); if (session.reconnect === reconnect) delete session.reconnect; };
  }, [session, pending, readOnly, gateway, report, withOwner, publish]);

  const save = useCallback(async (value: AgentContextState) => {
    if (!session) throw new Error("CONTEXT_CONFIGURATION_OWNER_UNAVAILABLE");
    if (session.readOnly) throw new Error("AGENT_CONTEXT_READ_ONLY");
    session.saves++;
    if (active(session)) setState(previous => ({ session, mode: session.mode, value: { ...(previous.session === session && previous.mode === session.mode ? previous.value : idle), saving: true } }));
    try { await withOwner(session, async owner => { await owner.edit(value); await owner.save(); await catchUpDeferred(session, owner); }); }
    catch (reason) { if (session.lease) publish(session); else report(session, reason); throw reason; }
    finally { session.saves--; publish(session); }
  }, [session, active, withOwner, publish, report, catchUpDeferred]);
  const retry = useCallback(async () => {
    if (!session || session.readOnly) throw new Error("AGENT_CONTEXT_READ_ONLY");
    try {
      const restoringSubscription = session.subscriptionFailed;
      let refreshed = false;
      if (restoringSubscription) {
        if (!session.reconnect) throw new Error("CONTEXT_CONFIGURATION_SUBSCRIPTION_UNAVAILABLE");
        await session.reconnect();
      }
      if (session.seedPending && session.startup) { await session.startup.retrySeed(() => !session.readOnly); session.seedPending = false; }
      else if (!session.lease && session.startup) await session.startup.repairLocal();
      await withOwner(session, async owner => {
        const snapshot = owner.getSnapshot();
        if (snapshot.session.preservationError) await owner.repairLocal();
        else if (snapshot.originalPending || snapshot.session.pending || snapshot.session.queued) await owner.retrySave();
        else if (!snapshot.session.conflicts.length) { await owner.refresh(); refreshed = true; }
        if (restoringSubscription) {
          const restored = owner.getSnapshot();
          if (!restored.originalPending && !restored.session.pending && !restored.session.queued && !restored.session.preservationError && !restored.session.conflicts.length
            && (snapshot.originalPending || snapshot.session.pending || snapshot.session.queued || snapshot.session.preservationError)) { await owner.refresh(); refreshed = true; }
        }
        await catchUpDeferred(session, owner);
      });
      if (restoringSubscription) { session.subscriptionPending = false; session.subscriptionFailed = false; delete session.subscriptionFailure; }
      if (refreshed) { session.refreshFailed = false; delete session.refreshFailure; }
      publish(session);
    } catch (reason) {
      const snapshot = session.lease?.owner.getSnapshot();
      if (snapshot && (snapshot.originalPending || snapshot.session.pending || snapshot.session.queued || snapshot.session.preservationError || snapshot.session.conflicts.length)) publish(session);
      else report(session, reason);
      throw reason;
    }
  }, [session, withOwner, publish, report, catchUpDeferred]);
  const resolve = useCallback(async (conflictId: string, action: "incoming" | "keep") => {
    if (!session || session.readOnly) throw new Error("AGENT_CONTEXT_READ_ONLY");
    await withOwner(session, async owner => { await owner.resolveConflict(conflictId, action); await owner.save(); await catchUpDeferred(session, owner); });
    publish(session);
  }, [session, withOwner, publish, catchUpDeferred]);
  const repair = useCallback(async () => {
    if (!session) throw new Error("CONTEXT_CONFIGURATION_OWNER_UNAVAILABLE");
    try {
      if (session.lease) await withOwner(session, async owner => { await owner.repairLocal(); await catchUpDeferred(session, owner); });
      else if (session.startup) await session.startup.repairLocal();
      publish(session);
    } catch (reason) { report(session, reason); throw reason; }
  }, [session, publish, report, catchUpDeferred, withOwner]);
  return { ...(state.session === session && state.mode === mode ? state.value : idle), save, retry, resolve, repair };
}
