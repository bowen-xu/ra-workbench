// SPDX-License-Identifier: AGPL-3.0-or-later
// Preview-only browser/background ports. No production UI or transport is replaced.
namespace ConnectorPreview {
  interface PreviewMessage { type: string; selected?: string[]; height?: number }
  interface LocalizedString { message: string; placeholders?: Record<string, { content: string }> }
  const context = window as Window & { connectorPreviewStrings?: Record<string, LocalizedString> };
  const params = new URLSearchParams(location.search);
  const mode = ConnectorPreview.scenario(params.get("scenario"));
  const selected = ConnectorPreview.selectedPapers(params.get("items"));
  const listeners = new Set<(request: unknown[], sender: object, respond: (value: unknown) => void) => void>();
  const timers = new Set<number>();
  const state: Record<string, unknown> = {
    researchAssistantPrivate: { schemaVersion: 2, environment: "development", receiverId: "a".repeat(64) },
  };
  function notify(message: PreviewMessage): void {
    if (parent !== window) parent.postMessage({ channel: "connector-preview", ...message }, location.origin);
  }
  function later(callback: () => void, delayMs: number): void {
    const id = window.setTimeout(() => { timers.delete(id); callback(); }, delayMs);
    timers.add(id);
  }
  function emit(name: string, args: unknown[]): void {
    for (const listener of listeners) listener([`progressWindowIframe.${name}`, args], {}, () => undefined);
  }
  function updatePdf(progress: number | false): void {
    for (const paper of selected) emit("updateProgress", [`${paper.id}-pdf`, {
      title: "Full Text PDF", parentItem: paper.id, progress,
      iconSrc: "/extension/images/treeitem-attachment-pdf.png", itemType: "attachment",
    }]);
  }
  function download(): void {
    updatePdf(15);
    later(() => updatePdf(52), 650);
    later(() => { updatePdf(100); notify({ type: "saved" }); }, 1400);
  }
  function initializeProgress(): void {
    const unavailable = mode === "offline" || mode === "conflict" || mode === "incompatible";
    emit("changeHeadline", [unavailable ? "桌面未连接" : "Saving to", {
      id: "L1", name: "Research Assistant 文献库", libraryID: 1, filesEditable: true,
    }, null, {}]);
    if (unavailable) {
      const error = mode === "conflict" ? "本机端口上的服务不是可识别的 Research Assistant，可能被其他软件占用。已停止传输。"
        : mode === "incompatible" ? "桌面与插件协议不兼容，已停止传输。请更新到兼容版本后重试。"
          : "无法连接 Research Assistant，请确认桌面应用已启动。";
      emit("addError", ["connectionError", error]);
      return;
    }
    for (const paper of selected) emit("updateProgress", [paper.id, {
      title: paper.title, progress: 100, iconSrc: "/extension/images/treeitem-journalArticle.png",
      itemType: "journalArticle", itemsLoaded: true,
    }]);
    if (mode === "replay") download();
    else updatePdf(mode === "failed" ? false : mode === "downloading" ? 42 : 100);
  }
  async function sendMessage(request: [string, unknown[]]): Promise<unknown> {
    const [method, args] = request;
    if (method === "Connector.getPref") return false;
    if (method === "Debug.log") return undefined;
    if (method === "selectDone") {
      const items = args[1];
      notify({ type: "selection", selected: items && typeof items === "object" ? Object.keys(items) : [] });
      return undefined;
    }
    if (method === "Messaging.sendMessage") {
      const [name, value] = args;
      if (name === "progressWindowIframe.registered") later(initializeProgress, 0);
      else if (name === "progressWindowIframe.resized") notify({ type: "resize", height: (value as { height: number }).height });
      else if (name === "progressWindowIframe.updated" && (value as { retryAttachments?: boolean })?.retryAttachments) download();
      return undefined;
    }
    throw new Error(`Preview has no browser adapter for ${method}`);
  }
  function getMessage(name: string, substitutions: string[] | string = []): string {
    const entry = context.connectorPreviewStrings?.[name];
    if (!entry) return "";
    const values = Array.isArray(substitutions) ? substitutions : [substitutions];
    return entry.message.replace(/\$([a-z_]+)\$/gi, (match, key: string) => entry.placeholders?.[key.toLowerCase()]?.content ?? match)
      .replace(/\$(\d+)/g, (_match, index: string) => values[Number(index) - 1] ?? "");
  }
  const browserPort = {
    runtime: {
      getManifest: () => ({ version: "0.1.0" }),
      getURL: (file: string) => `${location.origin}/extension/${file}`,
      sendMessage,
      onMessage: { addListener: (listener: typeof listeners extends Set<infer T> ? T : never) => { listeners.add(listener); } },
    },
    storage: { local: {
      get: async (key: string) => structuredClone({ [key]: state[key] }),
      set: async (values: Record<string, unknown>) => { Object.assign(state, structuredClone(values)); },
      setAccessLevel: async () => undefined,
    } },
    i18n: { getMessage },
    windows: { getCurrent: async () => ({ id: 1 }), remove: async () => undefined },
  };
  Object.assign(window, { chrome: browserPort, browser: browserPort });
  // The actual options/transport code calls fetch; every request is handled here.
  window.fetch = async (input, init) => {
    const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url);
    if (!/^http:\/\/127\.0\.0\.1:2312[01]$/.test(url.origin) || !["/connector/ping", "/connector/saveItems"].includes(url.pathname)) {
      throw new Error("Preview blocked a request outside its mock desktop port");
    }
    if (mode === "offline") throw new TypeError("Mock desktop offline");
    if (mode === "conflict") return new Response("<!doctype html><title>Another local application</title>", { headers: { "Content-Type": "text/html" } });
    const responseHeaders = { "Content-Type": "application/json", "X-Research-Assistant-Version": "0.2.0",
      "X-Research-Assistant-Application": "research-assistant", "X-Research-Assistant-Connector-API-Version": mode === "incompatible" ? "1" : "2",
      "X-Zotero-Connector-API-Version": "3", "X-Research-Assistant-Instance-ID": "b".repeat(32) };
    if (url.pathname === "/connector/ping") return new Response(JSON.stringify({
      application: "research-assistant", version: "0.2.0", connectorApiVersion: 2, zoteroConnectorApiVersion: 3,
      environment: url.port === "23121" ? "development" : "production", receiverId: "a".repeat(64),
      instanceId: "b".repeat(32), connectionToken: "0".repeat(64), prefs: {},
    }), { headers: responseHeaders });
    const token = new Headers(init?.headers).get("X-Research-Assistant-Token");
    return new Response(JSON.stringify(token === "0".repeat(64) ? { items: [] } : { error: "connection_changed" }), {
      status: token === "0".repeat(64) ? 200 : 401,
      headers: responseHeaders,
    });
  };
  window.addEventListener("pagehide", () => { for (const timer of timers) clearTimeout(timer); timers.clear(); listeners.clear(); });
}
