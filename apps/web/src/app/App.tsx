import {useEffect, useState} from "react";
import {WorkbenchApp} from "./WorkbenchApp";
import {assembleWorkbench} from "./workbenchComposition";
import {createRuntimePluginWorkbench, type RuntimePluginWorkbench} from "./runtime/plugins/runtimePluginWorkbench";
import {loadBrowserPlugins} from "./runtime/plugins/loadBrowserPlugins";
import {getAppStorage} from "../core/runtime/appStorage";
import {notify} from "../core/services/notifications";
import type {WorkspaceLocation} from "../core/contracts/workbench";

const composition = assembleWorkbench();

/** Browser development and previews use the same registration host and plugin
 * activation entries as independently built desktop packages. */
export function App(props: {readonly initialChatOpen?: boolean; readonly initialLocation?: WorkspaceLocation} = {}) {
  const [workbench, setWorkbench] = useState<RuntimePluginWorkbench>();
  useEffect(() => {
    const current = createRuntimePluginWorkbench({storage: getAppStorage()});
    let active = true;
    setWorkbench(current);
    void loadBrowserPlugins().then(result => active ? current.activate(result) : undefined)
      .catch((reason: unknown) => {if (active) notify.error(reason instanceof Error ? reason.message : String(reason));})
      .finally(current.completeInitialLoad);
    return () => {
      active = false;
      void current.dispose().catch((reason: unknown) => notify.error(reason instanceof Error ? reason.message : String(reason)));
    };
  }, []);
  return workbench ? <WorkbenchApp composition={composition} runtimePlugins={workbench} {...props}/> : null;
}
