// SPDX-License-Identifier: AGPL-3.0-or-later
Zotero.Inject.checkActionToServer = async () => {
  try {
    await Zotero.Connector.callMethod("ping", {});
    return true;
  } catch (error) {
    await Zotero.Inject.confirm({
      title: "Research Assistant 未连接",
      message: error instanceof Error ? error.message : "请启动桌面应用，并在插件设置中重新检测连接。",
      button1Text: "知道了",
      button2Text: "",
    });
    return false;
  }
};
