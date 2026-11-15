import { createContext, useContext, type ReactNode } from "react";

/** Captures the navigation owner before a potentially asynchronous operation. */
export type CaptureAgentNavigation = () => () => boolean;
const alwaysCurrent: CaptureAgentNavigation = () => () => true;
const Context = createContext<CaptureAgentNavigation>(alwaysCurrent);

export function AgentCanvasNavigationGuardProvider({capture, children}: {readonly capture: CaptureAgentNavigation; readonly children: ReactNode}): React.JSX.Element {
  return <Context.Provider value={capture}>{children}</Context.Provider>;
}
export function useCaptureAgentNavigation(): CaptureAgentNavigation {return useContext(Context);}
