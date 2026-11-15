import { createContext, useCallback, useContext, useId, useLayoutEffect, useRef, type ReactNode } from "react";
import { Bot, MessageCircle } from "lucide-react";
import "./AgentActivity.css";

const ActivityContext = createContext<((id: string, active: boolean) => void) | null>(null);

/** An idle or unmounted panel must not clear another mounted runtime's activity. */
export function AgentActivityProvider({onChange, children}: {
  readonly onChange: (active: boolean) => void;
  readonly children: ReactNode;
}): React.JSX.Element {
  const owners = useRef(new Set<string>());
  const publish = useCallback((id: string, active: boolean) => {
    const previous = owners.current.size > 0;
    if (active) owners.current.add(id); else owners.current.delete(id);
    const next = owners.current.size > 0;
    if (previous !== next) onChange(next);
  }, [onChange]);
  return <ActivityContext.Provider value={publish}>{children}</ActivityContext.Provider>;
}

export function usePublishAgentActivity(active: boolean): void {
  const publish = useContext(ActivityContext), id = useId();
  useLayoutEffect(() => {publish?.(id, active);}, [publish, id, active]);
  useLayoutEffect(() => () => publish?.(id, false), [publish, id]);
}

export function AgentActivitySpinner({size, strokeWidth = 2.5}: {readonly size: number; readonly strokeWidth?: number}): React.JSX.Element {
  return <svg className="agent-launcher-spinner" aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={strokeWidth}>
    <circle className="agent-spinner-track" cx={12} cy={12} r={9.5}/>
    <circle className="agent-spinner-arc" cx={12} cy={12} r={9.5} pathLength={100} strokeDasharray="75 25" strokeLinecap="round"/>
  </svg>;
}
export function AgentActivityIcon({active}: {readonly active: boolean}): React.JSX.Element {
  return active ? <AgentActivitySpinner size={22}/>
    : <MessageCircle aria-hidden="true" size={22}/>;
}
export function InternalConversationIcon({active}: {readonly active: boolean}): React.JSX.Element {
  return <span className="agent-internal-activity" aria-hidden="true"><Bot size={active ? 14 : 18}/>
    {active && <AgentActivitySpinner size={23} strokeWidth={1.5}/>}</span>;
}

export function agentLauncherDescription(active: boolean, referenceCount: number): string | undefined {
  return [active && "正在处理消息", referenceCount > 0 && `草稿含 ${referenceCount} 项引用`].filter(Boolean).join("；") || undefined;
}
