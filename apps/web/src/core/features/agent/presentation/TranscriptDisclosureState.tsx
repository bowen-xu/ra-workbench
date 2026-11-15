import {createContext, useContext, useEffect, useMemo, useState, type ReactNode} from "react";

const context = createContext<{readonly scope: string; readonly states: Map<string, boolean>} | null>(null);

/** Presentation state outlives recycled DOM, but never becomes a history fact. */
export function TranscriptDisclosureState({scope, states, children}: {
  readonly scope: string; readonly states: Map<string, boolean>; readonly children: ReactNode;
}): React.JSX.Element {
  const value = useMemo(() => ({scope, states}), [scope, states]);
  return <context.Provider value={value}>{children}</context.Provider>;
}

export function useTranscriptDisclosure(identity: string | undefined, initial = false) {
  const owner = useContext(context);
  const key = owner && identity ? `${owner.scope}/${identity}` : null;
  const [open, setOpen] = useState(() => key ? owner!.states.get(key) ?? initial : initial);
  useEffect(() => {if (key) owner!.states.set(key, open);}, [owner?.states, key, open]);
  return [open, setOpen] as const;
}
