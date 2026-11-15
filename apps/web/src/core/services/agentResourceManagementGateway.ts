import { hasPlatformCapabilities } from "../runtime/appRuntime";
import { invoke } from "../runtime/platformCore";
import { open } from "../runtime/platformDialog";

export type AgentSkillImportSource =
  | {readonly kind: "local"; readonly directory: string}
  | {readonly kind: "git"; readonly url: string; readonly ref: string | null; readonly subdirectory: string | null};
export interface AgentSkillImportCandidate {
  readonly sourceId: string;
  readonly name: string;
  readonly revision: string;
  readonly existingRevision: string | null;
}
export interface AgentSkillImportPreview {
  readonly importId: string;
  readonly candidates: readonly AgentSkillImportCandidate[];
  readonly resolvedCommit: string | null;
}
export interface AgentExecutionHost {
  readonly conversationId: string;
  readonly title: string;
  readonly kind: string;
  readonly resourceId: string;
  readonly approved: boolean;
}
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Agent 资源响应无效");
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  if (typeof value !== "string" || value.length === 0 || value.length > 4096 || /[\u0000-\u001f]/u.test(value)) throw new Error("Agent 资源身份无效");
  return value;
}
export function decodeSkillImportPreview(value: unknown): AgentSkillImportPreview {
  const record = object(value);
  if (!Array.isArray(record.candidates) || record.candidates.length > 256) throw new Error("技能候选响应无效");
  const candidates = record.candidates.map(value => {
    const row = object(value);
    return {sourceId: text(row.sourceId), name: text(row.name), revision: text(row.revision), existingRevision: row.existingRevision === null ? null : text(row.existingRevision)};
  });
  if (new Set(candidates.map(row => row.sourceId)).size !== candidates.length) throw new Error("技能候选身份重复");
  const resolvedCommit = record.resolvedCommit === null ? null : text(record.resolvedCommit);
  if (resolvedCommit !== null && !/^[a-f0-9]{40}$/u.test(resolvedCommit)) throw new Error("Git 提交身份无效");
  return {importId: text(record.importId), candidates, resolvedCommit};
}
function decodeHosts(value: unknown): readonly AgentExecutionHost[] {
  if (!Array.isArray(value) || value.length > 256) throw new Error("聊天执行授权响应无效");
  return value.map(value => {
    const row = object(value);
    if (typeof row.approved !== "boolean" || !["paper", "note", "readingNote"].includes(String(row.kind))) throw new Error("聊天执行授权身份无效");
    return {conversationId: text(row.conversationId), title: text(row.title), kind: text(row.kind), resourceId: text(row.resourceId), approved: row.approved};
  });
}
async function call(command: string, args: Record<string, unknown> = {}): Promise<unknown> {
  if (!hasPlatformCapabilities()) throw new Error("资源管理需要桌面应用");
  return invoke<unknown>(command, args);
}
async function mutate(command: string, args: Record<string, unknown>): Promise<void> { await call(command, args); }
export const agentResourceManagementGateway = {
  chooseSkillDirectory: async (): Promise<string | null> => {
    if (!hasPlatformCapabilities()) throw new Error("资源管理需要桌面应用");
    const selected = await open({directory: true, multiple: false, title: "选择技能目录"});
    return typeof selected === "string" ? selected : null;
  },
  executionHosts: async () => decodeHosts(await call("agent_list_execution_hosts")),
  authorizeShell: (conversationId: string, approved: boolean) => mutate("agent_set_host_shell_authorization", {conversationId, approved}),
  scan: async (source: AgentSkillImportSource) => decodeSkillImportPreview(await call("agent_scan_skill_import", {source})),
  install: (importId: string, selections: readonly {readonly sourceId: string; readonly expectedExistingRevision: string | null}[]) => mutate("agent_install_skill_import", {importId, selections}),
  cancel: (importId: string) => mutate("agent_cancel_skill_import", {importId}),
  remove: (sourceId: string, revision: string) => mutate("agent_remove_skill", {sourceId, revision}),
  readMcp: async (): Promise<string> => {
    const value = await call("agent_read_mcp_configuration");
    if (typeof value !== "string" || new TextEncoder().encode(value).length > 262_144) throw new Error("MCP 配置响应无效");
    return value;
  },
  saveMcp: (text: string, expectedText: string) => mutate("agent_save_mcp_configuration", {text, expectedText}),
  credential: (handle: string, secret: string | null) => mutate("agent_set_mcp_credential", {handle, secret}),
};
