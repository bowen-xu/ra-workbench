import { hasPlatformCapabilities } from "../runtime/appRuntime";
import { invoke } from "../runtime/platformCore";
import type { AgentResourceScope, AgentSkillIdentity } from "@research-assistant/core/agent";

export async function readAgentSkillDocument(identity: AgentSkillIdentity, scope?: AgentResourceScope): Promise<string> {
  if (!hasPlatformCapabilities()) throw new Error("技能说明需要桌面应用");
  const body = await invoke<unknown>("agent_read_skill_document", { identity, scope: scope ?? null });
  if (typeof body !== "string" || body.includes("\0") || new TextEncoder().encode(body).length > 1_048_576) throw new Error("技能说明响应无效");
  return body;
}
