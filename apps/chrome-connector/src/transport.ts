// SPDX-License-Identifier: AGPL-3.0-or-later
// Keep Zotero's capture/download pipeline; verify the local receiver before data transfer.
namespace ResearchAssistant {
  export interface RequestOptions {
    method: string;
    headers?: Record<string, string>;
    queryString?: string;
    timeout?: number;
  }
  const methods = new Set(["ping", "getSelectedCollection", "saveItems", "saveAttachment",
    "saveStandaloneAttachment", "saveSnapshot", "getRecognizedItem", "hasAttachmentResolvers",
    "delaySync", "updateSession", "sessionStatus"]);
  const privateFields = new Set(["cookie", "cookies", "detailedcookies", "cookiesandbox", "authorization", "headers"]);
  export function sanitize(value: unknown): unknown {
    if (Array.isArray(value)) return value.map(sanitize);
    if (value && typeof value === "object") return Object.fromEntries(Object.entries(value)
      .filter(([key]) => !privateFields.has(key.toLowerCase())).map(([key, entry]) => [key, sanitize(entry)]));
    return value;
  }
  export function createTransport(storage: LocalStorage, fetcher: typeof fetch, version: string) {
    let queue = Promise.resolve();
    async function updateConfig(environment: Environment, change: (config: ConnectionConfig) => void): Promise<void> {
      const operation = queue.then(async () => {
        const config = await ResearchAssistant.loadConnection(storage);
        if (config.environment !== environment) throw new ResearchAssistant.CommunicationError("连接设置已变化，请回到原论文页面重新采集。", 409);
        change(config);
        await storage.set({ [ResearchAssistant.STORAGE_KEY]: config });
      });
      queue = operation.catch(() => undefined);
      return operation;
    }
    return async function request(options: RequestOptions | string, data: unknown = null): Promise<unknown> {
      if (typeof options === "string") options = { method: options };
      if (!methods.has(options.method)) throw new ResearchAssistant.CommunicationError("此桌面连接器不支持请求的功能。", 501);
      const query = new URLSearchParams(options.queryString);
      if ([...query.keys()].some(key => key !== "sessionID")) throw new ResearchAssistant.CommunicationError("无效的采集会话参数。", 400);
      const binary = data instanceof ArrayBuffer;
      if (binary && data.byteLength > 64 * 1024 * 1024) throw new ResearchAssistant.CommunicationError("PDF 超过 64 MiB 限制，题录已保留。", 413);
      let metadata: string | undefined;
      if (options.headers?.["X-Metadata"]) {
        let value: unknown;
        try { value = JSON.parse(options.headers["X-Metadata"]); } catch { throw new ResearchAssistant.CommunicationError("无效的 PDF 元数据。", 400); }
        if (!value || typeof value !== "object" || Array.isArray(value)) throw new ResearchAssistant.CommunicationError("无效的 PDF 元数据。", 400);
        const fields = new Set(["id", "url", "contentType", "parentItemID", "title"]);
        metadata = JSON.stringify(Object.fromEntries(Object.entries(value).filter(([key]) => fields.has(key))));
      }
      const config = await ResearchAssistant.loadConnection(storage);
      const payload = binary ? data : sanitize(data);
      // Every call probes again. A cached success must not authorize a later PDF.
      const receiver = await ResearchAssistant.probeConnection(fetcher, config, version);
      await updateConfig(config.environment, current => {
        if ((config.receiverId && config.receiverId !== receiver.receiverId)
          || (current.receiverId && current.receiverId !== receiver.receiverId)) {
          throw new ResearchAssistant.CommunicationError("该地址现在连接到另一个文献库。为避免误存，已停止传输；请切回原桌面文献库后重新采集。", 409);
        }
        current.receiverId = receiver.receiverId;
      });
      if (options.method === "ping") {
        // Keep connection tokens out of upstream RPCs, preferences and UI text.
        const { connectionToken: _privateToken, ...status } = receiver;
        return status;
      }
      const headers = { ...ResearchAssistant.requestHeaders(version),
        "Content-Type": binary ? "application/pdf" : "application/json",
        "X-Research-Assistant-Instance-ID": receiver.instanceId,
        "X-Research-Assistant-Token": receiver.connectionToken,
        ...(metadata ? { "X-Metadata": metadata } : {}),
      };
      const url = `${ResearchAssistant.endpoint(config.environment)}connector/${options.method}${query.toString() ? `?${query}` : ""}`;
      let response: Response;
      try {
        response = await fetcher(url, { method: data === null ? "GET" : "POST", headers,
          body: data === null ? undefined : binary ? data : JSON.stringify(payload),
          signal: AbortSignal.timeout(options.timeout || 60_000), credentials: "omit", redirect: "error", cache: "no-store" });
      } catch {
        throw new ResearchAssistant.CommunicationError("连接已中断，无法确认保存结果。请重新检测，并回到原论文页面重新采集。", 503);
      }
      ResearchAssistant.validateResponse(response, receiver.instanceId);
      const result = await ResearchAssistant.readJson(response);
      if (!response.ok) throw new ResearchAssistant.CommunicationError(response.status === 401 || response.status === 409
        ? "桌面连接已变化，请重新检测后重试。已完成的题录与附件仍保留。"
        : "桌面未能完成采集。题录与附件分别保留已完成结果，可重试。", response.status);
      return result;
    };
  }
}
