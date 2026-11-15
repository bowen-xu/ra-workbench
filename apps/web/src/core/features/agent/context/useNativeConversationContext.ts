import { useCallback, useEffect, useRef, useState } from "react";
import { EMPTY_AGENT_CONTEXT, type AgentContextState } from "@research-assistant/core/agent";
import type { AgentContextGateway } from "../../../services/agentContextGateway";
import { hasConversationContext, readConversationContext, writeConversationContext } from "./contextStorage";
import { useManagedConversationContext } from "./useManagedConversationContext";
interface Owner {
  readonly id: string | null;
  readonly gateway: AgentContextGateway;
  revision: number | null;
  operation: Promise<void>;
  mounted: boolean;
  readOnly: boolean;
}
/** Native authority wins over the client cache, including intentionally empty selections. */
function useLegacyConversationContext(id: string | null, pending: boolean, gateway: AgentContextGateway, changed: (state: AgentContextState) => void, failed: (reason: unknown) => void,
  {readOnly = false}: {readonly readOnly?: boolean} = {}) {
  const owner = useRef<Owner>({
    id,
    gateway,
    revision: null,
    operation: Promise.resolve(),
    mounted: true,
    readOnly
  });
  if (owner.current.id !== id || owner.current.gateway !== gateway)
    owner.current = {
      id,
      gateway,
      revision: null,
      operation: Promise.resolve(),
      mounted: true,
      readOnly
    };
  const session = owner.current;
  session.readOnly = readOnly;
  const callbacks = useRef({ changed, failed });
  callbacks.current = { changed, failed };
  const [status, setStatus] = useState<{
    owner: Owner;
    readOnly: boolean;
    ready: boolean;
    loading: boolean;
    saving: boolean;
  }>({
    owner: session,
    readOnly,
    ready: pending || !gateway.readConfiguration,
    loading: !pending && !!gateway.readConfiguration,
    saving: false
  });
  const current = (): boolean => owner.current === session && session.mounted;
  useEffect(() => {
    let active = true;
    session.mounted = true;
    const initialReady = pending || !gateway.readConfiguration;
    setStatus({
      owner: session,
      readOnly,
      ready: initialReady,
      loading: !initialReady,
      saving: false
    });
    if (!id || initialReady)
      return () => {
        session.mounted = false;
      };
    const hydrate = async (): Promise<void> => {
      if (!active || !current()) return;
      let saved = await gateway.readConfiguration!(id, !readOnly && !session.readOnly && !hasConversationContext(id));
      session.revision = saved?.revision ?? 0;
      if (!active || !current())
        return;
      // A missing native row is the only case in which legacy preferences import.
      if (!saved && !session.readOnly && gateway.saveConfiguration)
        saved = await gateway.saveConfiguration(id, 0, readConversationContext(id));
      if (!active || !current())
        return;
      session.revision = saved?.revision ?? 0;
      // Missing archived configuration is a valid read result. Keep the legacy
      // display cache for restoration without importing or granting it here.
      const state = saved?.state ?? (readOnly ? readConversationContext(id) : EMPTY_AGENT_CONTEXT);
      if (saved || !readOnly) writeConversationContext(id, state);
      callbacks.current.changed(state);
      setStatus({
        owner: session,
        readOnly,
        ready: true,
        loading: false,
        saving: false
      });
    };
    session.operation = session.operation.then(hydrate).catch(reason => {
      if (active && current()) {
        setStatus({
          owner: session,
          readOnly,
          ready: false,
          loading: false,
          saving: false
        });
        callbacks.current.failed(reason);
      }
    });
    return () => {
      active = false;
      session.mounted = false;
    };
  }, [session, id, pending, gateway, readOnly]);
  useEffect(() => {
    if (!id || pending || !gateway.subscribeConfiguration || !gateway.readConfiguration)
      return;
    let active = true;
    const refresh = async (): Promise<void> => {
      const saved = await gateway.readConfiguration!(id, false);
      if (!active || !current() || !saved || saved.revision === session.revision)
        return;
      session.revision = saved.revision;
      writeConversationContext(id, saved.state);
      callbacks.current.changed(saved.state);
      setStatus(previous => previous.owner === session ? {
        ...previous,
        ready: true,
        loading: false
      } : previous);
    };
    const subscription = gateway.subscribeConfiguration(id, () => {
      const task = session.operation.then(refresh);
      session.operation = task.catch(reason => {
        if (active && current()) callbacks.current.failed(reason);
      });
    });
    void subscription.catch(reason => {
      if (active && current())
        callbacks.current.failed(reason);
    });
    return () => {
      active = false;
      void subscription.then(unlisten => unlisten()).catch(() => undefined);
    };
  }, [session, id, pending, gateway]);
  const save = useCallback(async (state: AgentContextState): Promise<void> => {
    if (!id)
      throw new Error("对话尚未创建");
    if (readOnly || session.readOnly) throw new Error("AGENT_CONTEXT_READ_ONLY");
    // The queue and revision belong to this exact conversation, even after a switch.
    const task = session.operation.then(async () => {
      // Archiving revokes queued writes; an already started native write keeps
      // its original result and remains subject to the backend archive check.
      if (session.readOnly) throw new Error("AGENT_CONTEXT_READ_ONLY");
      if (current())
        setStatus(previous => ({
          owner: session,
          readOnly,
          ready: previous.owner === session && previous.ready,
          loading: false,
          saving: true
        }));
      try {
        if (gateway.readConfiguration && gateway.saveConfiguration) {
          if (session.revision === null) {
            const saved = await gateway.readConfiguration(id, !hasConversationContext(id));
            session.revision = saved?.revision ?? 0;
          }
          if (session.readOnly) throw new Error("AGENT_CONTEXT_READ_ONLY");
          const saved = await gateway.saveConfiguration(id, session.revision, state);
          session.revision = saved.revision;
          writeConversationContext(id, saved.state);
          if (current()) {
            callbacks.current.changed(saved.state);
            setStatus({
              owner: session,
              readOnly,
              ready: true,
              loading: false,
              saving: false
            });
          }
        }
        else {
          writeConversationContext(id, state);
          if (current()) {
            callbacks.current.changed(state);
            setStatus({
              owner: session,
              readOnly,
              ready: true,
              loading: false,
              saving: false
            });
          }
        }
      }
      finally {
        if (current())
          setStatus(previous => previous.owner === session ? { ...previous, saving: false } : previous);
      }
    });
    session.operation = task.catch(() => undefined);
    return task;
  }, [session, id, gateway, readOnly]);
  const selected = status.owner === session && status.readOnly === readOnly ? status : {
    ready: false,
    loading: true,
    saving: false
  };
  return {
    ready: selected.ready,
    loading: selected.loading,
    saving: selected.saving,
    save
  };
}

export function useNativeConversationContext(id: string | null, pending: boolean, gateway: AgentContextGateway, changed: (state: AgentContextState) => void, failed: (reason: unknown) => void,
  {readOnly = false}: {readonly readOnly?: boolean} = {}) {
  const managed = !!gateway.configurationSave && !!gateway.configurationSeed;
  const legacy = useLegacyConversationContext(managed && !readOnly ? null : id, managed && !readOnly ? true : pending, gateway, changed, failed, {readOnly});
  const current = useManagedConversationContext(id, pending, gateway, changed, failed, readOnly, managed);
  return managed && !readOnly ? current : { ...legacy, recovery: null, conflicts: [], detail: undefined,
    retry: async () => undefined, resolve: async () => undefined, repair: async () => undefined };
}
