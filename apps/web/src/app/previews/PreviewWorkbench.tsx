import {useEffect, useState} from "react";
import {WorkbenchApp} from "../WorkbenchApp";
import {assembleWorkbench} from "../workbenchComposition";
import {createRuntimePluginWorkbench, type RuntimePluginWorkbench} from "../runtime/plugins/runtimePluginWorkbench";
import {getAppStorage} from "../../core/runtime/appStorage";
import {notify} from "../../core/services/notifications";
import type {WorkspaceLocation} from "../../core/contracts/workbench";
import type {RuntimePluginLoadResult} from "../../core/runtime/plugins/loader";

const composition = assembleWorkbench();

export interface PreviewWorkbenchProps {
  readonly loadPlugins: () => Promise<RuntimePluginLoadResult>;
  readonly initialChatOpen?: boolean;
  readonly initialLocation?: WorkspaceLocation;
}

/** Use the production Workbench and activation lifecycle with a caller-owned catalog. */
export function PreviewWorkbench({loadPlugins, ...props}: PreviewWorkbenchProps) {
  const [workbench, setWorkbench] = useState<RuntimePluginWorkbench>();
  useEffect(() => {
    const current = createRuntimePluginWorkbench({storage: getAppStorage()});
    let active = true;
    setWorkbench(current);
    void loadPlugins().then(result => active ? current.activate(result) : undefined)
      .catch((reason: unknown) => {if (active) notify.error(reason instanceof Error ? reason.message : String(reason));})
      .finally(current.completeInitialLoad);
    return () => {
      active = false;
      void current.dispose().catch((reason: unknown) => notify.error(reason instanceof Error ? reason.message : String(reason)));
    };
  }, [loadPlugins]);
  return workbench ? <WorkbenchApp composition={composition} runtimePlugins={workbench} {...props}/> : null;
}
