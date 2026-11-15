export interface McpServerDraft {
  readonly name: string;
  readonly transport: "stdio" | "streamable_http";
  readonly program: string;
  readonly directory: string;
  readonly url: string;
  readonly arguments: readonly string[];
  readonly environment: readonly { readonly key: string; readonly value: string }[];
  readonly credentialHandle: string;
}
function quote(value: string): string {
  if (!value || new TextEncoder().encode(value).length > 4096 || /[\u0000-\u001f\u007f-\u009f]/u.test(value)) throw new Error("配置项不能为空、包含控制字符或超过 4096 字节。");
  return JSON.stringify(value);
}
/** Append one explicit server table; keep the original document byte-for-byte for CAS. */
export function appendMcpServer(original: string, draft: McpServerDraft): string {
  const rows = [`[servers.${quote(draft.name.trim())}]`, 'environment_id = "local"', 'revision = "1"', `transport = ${quote(draft.transport)}`, 'enabled = true'];
  if (draft.credentialHandle.trim()) rows.push(`credential_handle = ${quote(draft.credentialHandle.trim())}`);
  if (draft.transport === "stdio") {
    const absolute = (path: string): boolean => /^(?:\/|[a-zA-Z]:[\\/]|\\\\[^\\]+\\[^\\]+)/u.test(path);
    if (!absolute(draft.program) || !absolute(draft.directory)) throw new Error("启动命令和工作目录必须使用绝对路径。");
    if (draft.arguments.length > 64 || draft.environment.length > 64) throw new Error("参数和环境变量最多各 64 项。");
    const keys = draft.environment.map(item => item.key.trim());
    if (new Set(keys).size !== keys.length) throw new Error("环境变量名称不能重复。");
    rows.push(`program = ${quote(draft.program)}`, `directory = ${quote(draft.directory)}`, `arguments = [${draft.arguments.map(quote).join(", ")}]`,
      `environment = { ${draft.environment.map(item => `${quote(item.key.trim())} = ${quote(item.value)}`).join(", ")} }`);
  } else {
    let url: URL;
    try { url = new URL(draft.url); } catch { throw new Error("请输入有效的 MCP 服务地址。"); }
    const local = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
    if ((url.protocol !== "https:" && !(url.protocol === "http:" && local)) || url.username || url.password || url.hash) throw new Error("服务地址需要 HTTPS（本机可用 HTTP），且不能包含账号、密码或片段。");
    rows.push(`url = ${quote(draft.url)}`);
  }
  const base = original.trim() ? original : "schema_version = 1\n";
  const text = `${base}${base.endsWith("\n") ? "\n" : "\n\n"}${rows.join("\n")}\n`;
  if (new TextEncoder().encode(text).length > 262_144) throw new Error("MCP 配置超过 256 KiB 上限。");
  return text;
}
