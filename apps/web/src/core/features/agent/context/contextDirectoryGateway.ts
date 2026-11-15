import { invoke, isDesktopRuntime } from "../../../runtime/platformCore";
import { getAppRuntime } from "../../../runtime/appRuntime";
export interface AgentContextDirectory { readonly manuscriptId: string; readonly title: string; readonly path: string }
async function call(command: string, args: Record<string, unknown>): Promise<unknown> {
  const runtime = getAppRuntime();
  if (runtime.mode === "preview" && runtime.invoke) return runtime.invoke(command, args);
  if (!isDesktopRuntime()) throw new Error("目录配置需要桌面应用");
  return invoke(command, args);
}
function decode(value: unknown): AgentContextDirectory | null {
  if (value === null) return null;
  if (!value || typeof value !== "object") throw new Error("目录配置无效");
  const row = value as Record<string, unknown>;
  if (typeof row.manuscriptId !== "string" || typeof row.title !== "string" || typeof row.path !== "string") throw new Error("目录配置无效");
  return { manuscriptId: row.manuscriptId, title: row.title, path: row.path };
}
export const contextDirectoryGateway = {
  read: async (conversationId: string) => decode(await call("agent_read_context_directory", { conversationId })),
  list: async () => { const value = await call("agent_list_context_directories", {}); if (!Array.isArray(value)) throw new Error("目录列表无效"); return value.map(decode).filter((item): item is AgentContextDirectory => item !== null); },
  set: async (conversationId: string, expectedDirectoryId: string | null, directoryId: string | null) => decode(await call("agent_set_context_directory", { request: { conversationId, expectedDirectoryId, directoryId } })),
};
