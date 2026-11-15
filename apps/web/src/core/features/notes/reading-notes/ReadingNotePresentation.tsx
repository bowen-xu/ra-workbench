import { createContext, useContext, type ComponentType, type ReactNode } from "react";
import type { ReadingNoteBlock } from "@research-assistant/core/notes";

export interface ReadingNoteActionsProps { readonly note: ReadingNoteBlock; }
interface Presentation { readonly ResearchActions?: ComponentType<ReadingNoteActionsProps>; }
const Context = createContext<Presentation>({});

/** Application composition supplies domain actions; the Core document keeps its
 * own source snapshot, editing, recovery and save lifetime without those actions.
 */
export function ReadingNotePresentationProvider({children, ...value}: Presentation & {readonly children: ReactNode}) {
  return <Context.Provider value={value}>{children}</Context.Provider>;
}
export function useReadingNoteResearchActions() { return useContext(Context).ResearchActions; }
