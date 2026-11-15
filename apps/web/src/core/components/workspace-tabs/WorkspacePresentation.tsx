import { createContext, useContext, type ReactNode } from "react";

import type { WorkspacePresentation } from "../../contracts/workbench";
export type { WorkspacePresentation } from "../../contracts/workbench";
const Context = createContext<WorkspacePresentation>("workspace");

/** Document tabs retain their original editor/storage owner without its management chrome. */
export function WorkspacePresentationProvider({ value, children }: {
  readonly value: WorkspacePresentation; readonly children: ReactNode;
}): React.JSX.Element {
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useWorkspacePresentation(): WorkspacePresentation { return useContext(Context); }
