import { createContext, useContext, useEffect, useId, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import { agentContextKey, type AgentContextSelection } from "@research-assistant/core/agent";
import { agentContextGateway } from "../../../services/agentContextGateway";
import { ContextRecommendationCache } from "./ContextRecommendationCache";
interface ContextObservation {
    readonly selection: AgentContextSelection | null;
    readonly active: boolean;
    readonly surface: "tab" | "floating";
}
const Context = createContext<{
    readonly candidates: readonly AgentContextSelection[];
    readonly suggestions: readonly AgentContextSelection[];
} | null>(null);
type PublishObservation = (id: string, value: AgentContextSelection | null, active: boolean, surface?: ContextObservation["surface"]) => void;
const PublicationContext = createContext<PublishObservation | null>(null);
const EMPTY_SELECTIONS: readonly AgentContextSelection[] = [];
const RecommendationContext = createContext<ContextRecommendationCache | null>(null);
const noSubscription = (): (() => void) => () => undefined;
const zeroVersion = (): number => 0;
/** UI observations are suggestions. They never modify a conversation's selection. */
export function AgentContextProvider({ children }: {
    readonly children: ReactNode;
}): React.JSX.Element {
    const [entries, setEntries] = useState<ReadonlyMap<string, ContextObservation>>(new Map());
    const [recommendations] = useState(() => new ContextRecommendationCache(agentContextGateway));
    useEffect(() => {
        const focus = () => recommendations.invalidate();
        const notes = (event: Event) => {
            const detail = (event as CustomEvent<{pageId?: string; revision?: number}>).detail;
            // Body saves invalidate the next visit, without starting a catalog
            // scan on every autosave in the currently open editor.
            recommendations.invalidate(candidate => candidate.resource.kind === "note" && (!detail?.pageId || candidate.resource.resourceId === detail.pageId), detail?.revision === undefined);
        };
        const writing = () => recommendations.invalidate(candidate => candidate.resource.kind === "manuscriptFile" || candidate.resource.kind === "manuscript");
        window.addEventListener("focus", focus);
        window.addEventListener("research-assistant:note-resources-changed", notes);
        window.addEventListener("research-assistant:writing-notes-changed", writing);
        return () => {window.removeEventListener("focus", focus); window.removeEventListener("research-assistant:note-resources-changed", notes); window.removeEventListener("research-assistant:writing-notes-changed", writing);};
    }, [recommendations]);
    const publish = useMemo(() => (id: string, value: AgentContextSelection | null, active: boolean, surface: ContextObservation["surface"] = "tab") => setEntries(previous => {
        const old = previous.get(id);
        if ((!old && !value && !active) || old && old.active === active && old.surface === surface && JSON.stringify(old.selection) === JSON.stringify(value)) return previous;
        const next = new Map(previous);
        next.delete(id);
        if (value || active)
            next.set(id, {selection: value, active, surface});
        return next;
    }), []);
    const value = useMemo(() => {
        const rows = [...entries.values()].reverse().sort((a, b) => Number(a.surface === "floating") - Number(b.surface === "floating") || Number(b.active) - Number(a.active));
        const tabs = rows.filter(item => item.active && item.surface === "tab");
        const active = tabs.length ? tabs : rows.filter(item => item.active);
        const unique = (items: readonly ContextObservation[]) => {
            const selections = new Map<string, AgentContextSelection>();
            for (const {selection} of items) if (selection && !selections.has(agentContextKey(selection.resource))) selections.set(agentContextKey(selection.resource), selection);
            return [...selections.values()];
        };
        // A fast floating lookup cannot become the automatic choice while the
        // actual active tab is still resolving. Keep both request owners alive.
        const awaitingTab = tabs.length > 0 && !tabs.some(item => item.selection);
        return {candidates: unique(active), suggestions: unique(awaitingTab ? rows.filter(item => item.surface === "tab") : rows)};
    }, [entries]);
    return <RecommendationContext.Provider value={recommendations}><PublicationContext.Provider value={publish}><Context.Provider value={value}>{children}</Context.Provider></PublicationContext.Provider></RecommendationContext.Provider>;
}
export function useAgentContextCandidates(): readonly AgentContextSelection[] { return useContext(Context)?.candidates ?? EMPTY_SELECTIONS; }
export function useAgentContextSuggestions(): readonly AgentContextSelection[] { return useContext(Context)?.suggestions ?? EMPTY_SELECTIONS; }
export function useAgentContextCandidate(candidate: AgentContextSelection | null, active = true, surface: ContextObservation["surface"] = "tab"): void {
    const publish = useContext(PublicationContext), id = useId(), key = JSON.stringify(candidate);
    const recommendations = useContext(RecommendationContext);
    const version = useSyncExternalStore(recommendations?.subscribe ?? noSubscription, recommendations?.getVersion ?? zeroVersion, zeroVersion);
    useEffect(() => {
        if (!publish) return;
        let current = true;
        const saved = recommendations?.peek(candidate);
        if (!candidate || saved) {
            publish(id, saved ?? null, candidate ? active : false, surface);
            return;
        }
        publish(id, null, active, surface);
        let release: (() => void) | undefined;
        if (recommendations && ["paper","note","manuscriptFile","manuscript"].includes(candidate.resource.kind)) {
            // Open projects and files resolve their managed group; @ still exposes exact files.
            let request: ReturnType<ContextRecommendationCache["acquire"]>;
            try {request = recommendations.acquire(candidate);}
            catch {publish(id, null, active, surface); return;}
            release = request.release;
            void request.promise.then(selection => {
                if (current && version === recommendations.getVersion() && recommendations.peek(candidate) === selection) {
                    publish(id, selection, active, surface);
                }
            }).catch(() => { if (current) publish(id, null, active, surface); });
        } else publish(id, candidate, active, surface);
        return () => { current = false; release?.(); };
    }, [publish, id, key, active, surface, recommendations, version]);
    useEffect(() => () => publish?.(id, null, false), [publish, id]);
}
