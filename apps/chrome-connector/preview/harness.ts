// SPDX-License-Identifier: AGPL-3.0-or-later
namespace ConnectorPreview {
  type View = "capture" | "selector" | "options";
  const get = <T extends HTMLElement>(id: string): T => {
    const element = document.getElementById(id);
    if (!element) throw new Error(`Preview element missing: ${id}`);
    return element as T;
  };
  const frame = get<HTMLIFrameElement>("production-ui");
  const stage = document.querySelector<HTMLElement>(".preview-stage")!;
  const stateControl = get<HTMLSelectElement>("scenario");
  const help = get<HTMLParagraphElement>("scenario-help");
  const status = get<HTMLOutputElement>("preview-status");
  const replay = get<HTMLButtonElement>("replay");
  const params = new URLSearchParams(location.search);
  let view: View = params.get("view") === "options" ? "options" : params.get("view") === "selector" ? "selector" : "capture";
  let currentScenario = ConnectorPreview.scenario(params.get("scenario"));
  let chosen: string[] = [ConnectorPreview.papers[0].id];
  const descriptions: Record<Scenario, string> = {
    success: "题录与 PDF 已保存。选择其他状态可体验下载进度和失败重试。",
    downloading: "题录已经保存，PDF 正在下载。此状态保留在中途，方便查看进度样式。",
    failed: "题录已经保存。点击进度窗中的“重试 PDF”，只重试附件。",
    offline: "桌面未连接时显示实际错误反馈，不显示已保存。",
    conflict: "本机端口返回其他软件的响应，只探测连接，不发送题录或 PDF。",
    incompatible: "检测到协议不兼容时停止传输，请更新兼容版本后回到原页面重新采集。",
    replay: "正在模拟题录保存与 PDF 下载，稍后显示完成结果。",
  };
  function render(): void {
    stage.dataset.view = view;
    frame.hidden = false;
    frame.style.removeProperty("height");
    for (const button of document.querySelectorAll<HTMLButtonElement>("[data-view]")) {
      button.setAttribute("aria-pressed", String(button.dataset.view === view));
    }
    document.querySelector<HTMLElement>(".scenario-control")!.hidden = view === "selector";
    stateControl.value = currentScenario === "replay" ? "downloading" : currentScenario;
    get<HTMLElement>("stage-address").textContent = view === "capture" ? "示例论文 · paper.example" : view === "options" ? "Research Assistant Connector · 插件设置" : "Research Assistant Connector · 文献选择";
    replay.textContent = view === "capture" ? "模拟采集" : "重置示例";
    help.textContent = view === "capture" ? descriptions[currentScenario] : view === "options" ? "自动检测桌面连接，无需配对码。可切换桌面未连接、端口冲突或协议不兼容，检查连接提示。" : "使用实际选择器勾选文献，确认后将进入这些文献的采集进度。";
    status.textContent = "示例数据不会写入桌面文献库。";
    if (view === "options") frame.src = `/extension/research-assistant/options.html?scenario=${currentScenario}`;
    else if (view === "selector") frame.src = `/extension/itemSelector/itemSelector.html#${encodeURIComponent(JSON.stringify([1, Object.fromEntries(ConnectorPreview.papers.map(paper => [paper.id, paper.title]))]))}`;
    else frame.src = `/extension/progressWindow/progressWindow.html?scenario=${currentScenario}&items=${chosen.join(",")}`;
    const query = new URLSearchParams({ view, scenario: currentScenario });
    history.replaceState(null, "", `/?${query}`);
  }
  for (const button of document.querySelectorAll<HTMLButtonElement>("button[data-view]")) {
    button.addEventListener("click", () => { view = button.dataset.view as View; render(); });
  }
  stateControl.addEventListener("change", () => { currentScenario = ConnectorPreview.scenario(stateControl.value); render(); });
  replay.addEventListener("click", () => { if (view === "capture") currentScenario = "replay"; render(); });
  window.addEventListener("message", (event: MessageEvent) => {
    if (event.origin !== location.origin || event.source !== frame.contentWindow || event.data?.channel !== "connector-preview") return;
    if (event.data.type === "resize" && view === "capture" && Number.isFinite(event.data.height)) {
      frame.style.height = `${Math.max(90, Math.min(420, event.data.height + 35))}px`;
    } else if (event.data.type === "selection") {
      const ids = Array.isArray(event.data.selected) ? event.data.selected : [];
      const selection = ConnectorPreview.papers.filter(paper => ids.includes(paper.id)).map(paper => paper.id);
      if (!selection.length) { frame.hidden = true; status.textContent = "已取消选择，没有添加文献。点击“重置示例”可重新选择。"; return; }
      chosen = selection;
      view = "capture"; currentScenario = "replay"; render();
      status.textContent = `已选择 ${chosen.length} 篇示例文献。`;
    } else if (event.data.type === "saved") {
      help.textContent = "模拟采集完成，题录与 PDF 均已保存。";
      status.textContent = "结果仅存在于当前预览，不连接桌面服务。";
    }
  });
  render();
}
