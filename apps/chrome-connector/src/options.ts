// SPDX-License-Identifier: AGPL-3.0-or-later
(() => {
  const get = <T extends HTMLElement>(id: string): T => {
    const element = document.getElementById(id);
    if (!element) throw new Error(`Missing options element: ${id}`);
    return element as T;
  };
  const form = get<HTMLFormElement>("connection-form");
  const environment = get<HTMLSelectElement>("environment");
  const status = get<HTMLOutputElement>("status");
  const request = ResearchAssistant.createTransport(chrome.storage.local, fetch, chrome.runtime.getManifest().version);
  let busy = false;
  const show = (message: string) => { status.textContent = message; };
  const showError = (error: unknown) => show(error instanceof ResearchAssistant.CommunicationError ? error.message : "连接操作未完成，请重新检测后重试。");
  function setBusy(value: boolean): void {
    busy = value;
    for (const control of document.querySelectorAll<HTMLButtonElement | HTMLSelectElement>("button, select")) control.disabled = value;
  }
  async function refresh(): Promise<ResearchAssistant.ConnectionConfig> {
    const config = await ResearchAssistant.loadConnection(chrome.storage.local);
    environment.value = config.environment;
    return config;
  }
  async function detect(): Promise<void> {
    if (busy) return;
    setBusy(true); show("正在检测桌面应用…");
    try {
      const previous = await ResearchAssistant.loadConnection(chrome.storage.local);
      const selected = environment.value as ResearchAssistant.Environment;
      ResearchAssistant.endpoint(selected);
      await chrome.storage.local.setAccessLevel({ accessLevel: "TRUSTED_CONTEXTS" });
      await chrome.storage.local.set({ [ResearchAssistant.STORAGE_KEY]: {
        ...previous, environment: selected,
        ...(previous.environment !== selected ? { receiverId: undefined } : {}),
      } });
      const receiver = await request("ping", {}) as { version: string };
      show(`已连接 Research Assistant ${receiver.version}。现在可以打开论文页面并点击插件按钮采集。`);
    } catch (error) { showError(error); }
    finally { try { await refresh(); } catch (error) { showError(error); } setBusy(false); }
  }
  form.addEventListener("submit", event => { event.preventDefault(); void detect(); });
  environment.addEventListener("change", () => { void detect(); });
  void refresh().then(detect).catch(showError);
})();
