import { createContext, useContext, type ReactNode } from "react";
import { AgentTransientFailure } from "./AgentFailureMessage";

export interface AgentCanvasNavigationError {
  readonly message: string;
  readonly retry: () => void;
}

const Context = createContext<AgentCanvasNavigationError | null>(null);

export function AgentCanvasNavigationErrorProvider({error, children}: {readonly error: AgentCanvasNavigationError | null; readonly children: ReactNode}): React.JSX.Element {
  return <Context.Provider value={error}>{children}</Context.Provider>;
}

export function AgentCanvasNavigationFailure(): React.JSX.Element | null {
  const error = useContext(Context);
  return error ? <AgentTransientFailure id="conversation-navigation" content={error.message} retry={error.retry}/> : null;
}
