import { createContext, useContext, useLayoutEffect, type ReactNode } from "react";
import { getAppRuntime, type AppRuntime } from "./appRuntime";
import { PreviewPersistenceMonitor } from "./PreviewPersistenceMonitor";
import { installBrowserLinkRouting } from "./browserLinkRouting";
import { ApplicationKeyboardPolicy } from "./ApplicationKeyboardPolicy";

const RuntimeContext = createContext<AppRuntime | null>(null);
export function AppRuntimeProvider({ runtime, children }: { readonly runtime: AppRuntime; readonly children: ReactNode }): React.JSX.Element {
  useLayoutEffect(() => installBrowserLinkRouting(document, runtime), [runtime]);
  return <RuntimeContext.Provider value={runtime}><ApplicationKeyboardPolicy /><PreviewPersistenceMonitor storage={runtime.storage} />{children}</RuntimeContext.Provider>;
}
export function useAppRuntime(): AppRuntime { return useContext(RuntimeContext) ?? getAppRuntime(); }
