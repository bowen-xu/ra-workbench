// SPDX-License-Identifier: AGPL-3.0-or-later
namespace ResearchAssistant {
  export const STORAGE_KEY = "researchAssistantPrivate";
  export const API_VERSION = "2";
  export type Environment = "production" | "development";
  export interface ConnectionConfig {
    schemaVersion: 2;
    environment: Environment;
    receiverId?: string;
  }
  export interface LocalStorage {
    get(key: string): Promise<Record<string, unknown>>;
    set(values: Record<string, unknown>): Promise<void>;
  }
  export interface Receiver {
    application: "research-assistant";
    version: string;
    connectorApiVersion: 2;
    zoteroConnectorApiVersion: 3;
    environment: Environment;
    receiverId: string;
    instanceId: string;
    connectionToken: string;
    prefs: Record<string, unknown>;
  }
  export class CommunicationError extends Error {
    readonly name = "Research Assistant Connector Error";
    constructor(message: string, readonly status = 503, readonly value: unknown = "") { super(message); }
  }
  export function endpoint(environment: Environment): string {
    if (environment === "production") return "http://127.0.0.1:23120/";
    if (environment === "development") return "http://127.0.0.1:23121/";
    throw new CommunicationError("请选择有效的桌面环境。", 400);
  }
  const isObject = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
  const hex = (value: unknown, length: number): value is string => typeof value === "string" && value.length === length && /^[a-f0-9]+$/.test(value);
  export async function loadConnection(storage: LocalStorage): Promise<ConnectionConfig> {
    const value = (await storage.get(STORAGE_KEY))[STORAGE_KEY];
    if (value === undefined) return { schemaVersion: 2, environment: "production" };
    if (!isObject(value)) throw new CommunicationError("插件连接设置无效，无法确认桌面连接。", 400);
    const environment = (value.environment ?? "production") as Environment;
    endpoint(environment);
    let receiverId = value.receiverId;
    // Preserve the first development build's receiver identity without retaining
    // its manual credential or the removed metadata queue on persistence.
    if (value.schemaVersion !== 2 && hex(value.token, 64)) {
      receiverId = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value.token))))
        .map(byte => byte.toString(16).padStart(2, "0")).join("");
    }
    if (receiverId !== undefined && !hex(receiverId, 64)) throw new CommunicationError("接收端记录损坏，请检查插件设置。", 400);
    return { schemaVersion: 2, environment, ...(receiverId ? { receiverId } : {}) };
  }
  export function requestHeaders(version: string): Record<string, string> {
    return { "Content-Type": "application/json", "X-Zotero-Version": version,
      "X-Zotero-Connector-API-Version": "3", "X-Research-Assistant-Connector-API-Version": API_VERSION };
  }
  export function validateResponse(response: Response, instanceId?: string): void {
    if (response.headers.get("X-Research-Assistant-Application") !== "research-assistant"
      || !/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(response.headers.get("X-Research-Assistant-Version") ?? "")
      || !/^application\/json(?:;|$)/i.test(response.headers.get("Content-Type") ?? "")) {
      void response.body?.cancel();
      throw new CommunicationError("本机端口上的服务不是可识别的 Research Assistant，可能被其他软件占用。已停止传输。", 502);
    }
    if (response.headers.get("X-Research-Assistant-Connector-API-Version") !== API_VERSION
      || response.headers.get("X-Zotero-Connector-API-Version") !== "3") {
      void response.body?.cancel();
      throw new CommunicationError("桌面与插件协议不兼容，已停止传输。请更新到兼容版本后重试。", 412);
    }
    if (instanceId && response.headers.get("X-Research-Assistant-Instance-ID") !== instanceId) {
      void response.body?.cancel();
      throw new CommunicationError("本机接收服务已变化，无法确认保存结果。请重新检测，并回到原论文页面重新采集。", 409);
    }
  }
  export async function readJson(response: Response, maximum = 1024 * 1024): Promise<unknown> {
    const reader = response.body?.getReader();
    if (!reader) throw new CommunicationError("本机服务返回了空响应，已停止传输。", 502);
    const chunks: Uint8Array[] = [];
    let size = 0;
    try {
      while (true) {
        const next = await reader.read();
        if (next.done) break;
        size += next.value.byteLength;
        if (size > maximum) { await reader.cancel(); throw new Error("Oversized response"); }
        chunks.push(next.value);
      }
      const bytes = new Uint8Array(size);
      let offset = 0;
      for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
      return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
    } catch {
      throw new CommunicationError("本机服务响应格式无效或超出限制，已停止传输。", 502);
    } finally { reader.releaseLock(); }
  }
  export async function probeConnection(fetcher: typeof fetch, config: ConnectionConfig, version: string): Promise<Receiver> {
    let response: Response;
    try {
      response = await fetcher(`${endpoint(config.environment)}connector/ping`, {
        method: "GET", headers: requestHeaders(version), signal: AbortSignal.timeout(3000),
        credentials: "omit", redirect: "error", cache: "no-store",
      });
    } catch {
      throw new CommunicationError("无法连接 Research Assistant。请确认桌面应用已启动；连接也可能被阻止或端口被其他软件占用。连接恢复后，请回到原论文页面重新采集。", 503);
    }
    validateResponse(response);
    if (response.status !== 200) { void response.body?.cancel(); throw new CommunicationError("本机服务当前不可用，已停止传输。", response.status); }
    const value = await readJson(response, 64 * 1024);
    if (!isObject(value) || value.application !== "research-assistant"
      || value.version !== response.headers.get("X-Research-Assistant-Version")
      || value.connectorApiVersion !== 2 || value.zoteroConnectorApiVersion !== 3
      || !hex(value.receiverId, 64) || !hex(value.instanceId, 32) || !hex(value.connectionToken, 64)
      || response.headers.get("X-Research-Assistant-Instance-ID") !== value.instanceId
      || !isObject(value.prefs)) {
      throw new CommunicationError("本机端口未返回完整的 Research Assistant 身份与协议，可能被其他软件占用。已停止传输，未发送文献或 PDF。", 502);
    }
    if (value.environment !== config.environment) throw new CommunicationError("本机服务运行环境不匹配，已停止传输。请检查正式／开发应用的选择。", 409);
    return value as unknown as Receiver;
  }
}
