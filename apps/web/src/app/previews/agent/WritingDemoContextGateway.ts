import { decodeAgentContextState, type AgentContextSelection } from "@research-assistant/core/agent";
import type { AgentSessionGateway } from "@research-assistant/data/agent";
import type { AgentContextConfiguration, AgentContextGateway } from "../../../core/services/agentContextGateway";

/** Mock configuration persistence only. The production context owner still
 * hydrates, imports legacy drafts and checks revisions before every send. */
export function createWritingDemoContextGateway(sessions: AgentSessionGateway, inventory?: {
  readonly manuscriptId: string;
  readonly catalog: () => Promise<readonly AgentContextSelection[]>;
}): AgentContextGateway {
  const saved = new Map<string, AgentContextConfiguration>();
  const unavailable = async (): Promise<never> => { throw new Error("写作预览通过稿件提交网关执行，不提供独立上下文执行。"); };
  const directories = new Map<string, string | null>();
  const directory = (id: string | null) => id ? {manuscriptId: id, title: "当前演示稿件", path: "演示写作项目"} : null;
  const search = async (query: string): Promise<readonly AgentContextSelection[]> => {
    const needle = query.trim().toLocaleLowerCase();
    return (await inventory?.catalog() ?? []).filter(item => [item.label, ...("parts" in item.resource ? item.resource.parts.map(part => part.label) : [])].some(label => label.toLocaleLowerCase().includes(needle)));
  };
  return {
    ...(inventory ? {directories: {
      read: async (id: string) => directory(directories.has(id) ? directories.get(id)! : inventory.manuscriptId),
      set: async (id: string, expected: string | null, next: string | null) => {
        const current = directories.has(id) ? directories.get(id)! : inventory.manuscriptId;
        if (current !== expected || (next !== null && next !== inventory.manuscriptId)) throw new Error("模拟写作项目身份不匹配");
        directories.set(id, next); return directory(next);
      },
    }} : {}),
    readConfiguration: async id => structuredClone(saved.get(id) ?? null),
    saveConfiguration: async (id, revision, state) => {
      if ((saved.get(id)?.revision ?? 0) !== revision) throw new Error("模拟上下文配置版本冲突");
      const next = {revision: revision + 1, state: decodeAgentContextState(structuredClone(state))};
      saved.set(id, next); return structuredClone(next);
    },
    sessions, list: async () => [], search, catalog: search,
    prepare: unavailable, restore: unavailable, start: unavailable, interrupt: unavailable,
  };
}
