import { createContext, useContext } from "react";

export const EditorReadOnlyContext = createContext(false);

/** Custom node-view controls must honor the same mode as the editable surface. */
export function useEditorReadOnly(): boolean {
  return useContext(EditorReadOnlyContext);
}
