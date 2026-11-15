import { createContext, useContext, type ReactNode } from "react";
import type { SpeechRecognitionConstructor } from "./speechRecognition";

/** Hosts may replace recognition and permission side effects without replacing the control. */
export interface SpeechInputAdapter {
  readonly recognition: SpeechRecognitionConstructor;
  readonly disclosure?: string;
}
const SpeechAdapterContext = createContext<SpeechInputAdapter | undefined>(undefined);
export function SpeechInputAdapterProvider({ adapter, children }: {
  readonly adapter: SpeechInputAdapter;
  readonly children: ReactNode;
}): React.JSX.Element {
  return <SpeechAdapterContext.Provider value={adapter}>{children}</SpeechAdapterContext.Provider>;
}
export function useSpeechInputAdapter(): SpeechInputAdapter | undefined {
  return useContext(SpeechAdapterContext);
}
