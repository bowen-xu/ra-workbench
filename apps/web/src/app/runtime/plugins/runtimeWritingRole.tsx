import {useCallback, useLayoutEffect, useMemo, useRef, useState, type ComponentType, type ReactNode} from "react";
import type {AgentDomainSessionProps} from "@research-assistant/workbench-sdk/agent-presentation";
import type {PluginWritingRuntimeBinding} from "@research-assistant/workbench-sdk/activation";
import type {RegisteredPluginView} from "../../../core/runtime/plugins/viewRegistry";
import {getReaderCloseCoordinator} from "../../../core/public/workbench";
import type {RuntimePluginWorkbench} from "./runtimePluginWorkbench";
import {RuntimePluginViewBoundary} from "./RuntimePluginViewBoundary";

interface Publication {
  readonly registration: RegisteredPluginView;
  readonly binding: PluginWritingRuntimeBinding;
  readonly token: object;
}
export interface RuntimeWritingRole {
  readonly registration: RegisteredPluginView | undefined;
  readonly binding: PluginWritingRuntimeBinding | undefined;
  readonly Session: ComponentType<AgentDomainSessionProps> | undefined;
  readonly host: ReactNode;
  readonly readBinding: (registration: RegisteredPluginView) => PluginWritingRuntimeBinding | undefined;
}
const ignoreFailure = () => undefined;

/** One presentation binding for the original writing registration. The file
 * owner remains private to the mounted Runtime and outlives either UI role. */
export function useRuntimeWritingRole(workbench: RuntimePluginWorkbench | undefined,
  views: readonly RegisteredPluginView[]): RuntimeWritingRole {
  const registration = views.find(view => view.pluginId === "writing_projects" && view.id === "writing" && view.writing);
  const [publication, setPublication] = useState<Publication>();
  const current = useRef<Publication | undefined>(undefined);
  const latest = useRef(registration); latest.current = registration;
  const readBinding = useCallback((registered: RegisteredPluginView) => {
    const value = current.current;
    return value?.registration === registered && latest.current === registered
      && workbench?.views.getSnapshot().includes(registered) ? value.binding : undefined;
  }, [workbench]);
  const onReady = useCallback((binding: PluginWritingRuntimeBinding): (() => void) => {
    if (!registration || latest.current !== registration
      || !workbench?.views.getSnapshot().includes(registration)) return ignoreFailure;
    if (!binding || typeof binding !== "object" || Reflect.ownKeys(binding).length !== 2
      || Reflect.ownKeys(binding).some(key => key !== "Scope" && key !== "prepareClose")
      || typeof binding.Scope !== "function" || typeof binding.prepareClose !== "function")
      throw Object.assign(new Error(`plugin_writing_binding_invalid: ${registration.pluginId} / ${registration.id}`), {code: "plugin_writing_binding_invalid"});
    const value: Publication = {registration, binding, token: {}};
    current.current = value; setPublication(value);
    return () => {
      if (current.current?.registration !== registration || current.current.token !== value.token) return;
      current.current = undefined;
      setPublication(previous => previous?.token === value.token ? undefined : previous);
    };
  }, [registration, workbench]);
  const runtimeFailed = useCallback((registered: RegisteredPluginView) => {
    if (current.current?.registration !== registered) return;
    current.current = undefined;
    setPublication(previous => previous?.registration === registered ? undefined : previous);
  }, []);
  const binding = publication?.registration === registration && registration ? readBinding(registration) : undefined;
  const Session = useMemo(() => {
    const Original = registration?.writing?.Session, Scope = binding?.Scope;
    if (!registration || !binding || !Original || !Scope || !workbench) return undefined;
    const recover = async () => {
      const available = () => readBinding(registration) === binding && workbench.getSnapshot()
        .some(status => status.pluginId === registration.pluginId && status.state === "ready");
      if (!available() || !await getReaderCloseCoordinator().prepareAll() || !available())
        throw new Error(`plugin_view_recovery_blocked: ${registration.pluginId} / ${registration.id}`);
    };
    return function BoundWritingSession(props: AgentDomainSessionProps) {
      return <Scope><RuntimePluginViewBoundary registration={registration} viewProps={{active: props.open}}
        renderContent={() => <Original {...props}/>} onFailure={ignoreFailure} onRecover={recover}/></Scope>;
    };
  }, [registration, binding, workbench, readBinding]);
  const host = registration && workbench ? <RuntimeWritingHost registration={registration} workbench={workbench} onReady={onReady} onFailure={runtimeFailed}/> : null;
  return {registration, binding, Session, host, readBinding};
}

function RuntimeWritingHost({registration, workbench, onReady, onFailure}: {
  readonly registration: RegisteredPluginView; readonly workbench: RuntimePluginWorkbench;
  readonly onReady: (binding: PluginWritingRuntimeBinding) => () => void;
  readonly onFailure: (registration: RegisteredPluginView) => void;
}) {
  const recover = useCallback(async () => {
    const available = () => workbench.views.getSnapshot().includes(registration)
      && workbench.getSnapshot().some(status => status.pluginId === registration.pluginId && status.state === "ready");
    if (!available() || !await getReaderCloseCoordinator().prepareAll() || !available())
      throw new Error(`plugin_view_recovery_blocked: ${registration.pluginId} / ${registration.id}`);
  }, [registration, workbench]);
  return <RuntimePluginViewBoundary registration={registration} viewProps={{active: false}} onFailure={onFailure} onRecover={recover}
    renderContent={() => <RuntimeWritingMount registration={registration} workbench={workbench} onReady={onReady}/>}/>;
}

/** The registration can survive a failed initial render. Its abandoned mount
 * cannot publish a scope into a later explicit retry of the same registration. */
function RuntimeWritingMount({registration, workbench, onReady}: {
  readonly registration: RegisteredPluginView; readonly workbench: RuntimePluginWorkbench;
  readonly onReady: (binding: PluginWritingRuntimeBinding) => () => void;
}) {
  const mounted = useRef(false);
  useLayoutEffect(() => {mounted.current = true; return () => {mounted.current = false;};}, []);
  const publish = useCallback((binding: PluginWritingRuntimeBinding) => mounted.current ? onReady(binding) : ignoreFailure, [onReady]);
  const Runtime = registration.writing!.Runtime;
  return <Runtime onReady={publish} subscribe={workbench.subscribe}
    isDisabled={() => !workbench.getSnapshot().some(status => status.pluginId === registration.pluginId && status.state === "ready")}/>;
}
