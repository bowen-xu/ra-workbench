// SPDX-License-Identifier: AGPL-3.0-or-later
// Every replacement is anchored and fails on drift; upstream source is never edited.
export function replaceOnce(source, before, after, label) {
  if (source.split(before).length !== 2) throw new Error(`Upstream drift: ${label}`);
  return source.replace(before, after);
}

export function patchUpstream(path, input, schema) {
  let source = input;
  const change = (before, after) => {
    source = replaceOnce(source, before, after, `${path}: ${before.slice(0, 65)}`);
  };
  if (path === "zotero.js") {
    for (const [name, value] of Object.entries({ isManifestV3: true, isFirefox: false, isSafari: false, allowRepoTranslatorTester: false })) {
      change(`/* this.${name} = SET IN BUILD SCRIPT */`, `this.${name} = ${value}`);
    }
    for (const [key, value] of Object.entries({
      automaticSnapshots: false, firstUse: false, firstSaveToServer: false,
      reportTranslationFailure: false, "integration.googleDocs.enabled": false,
      "proxies.transparent": false, "proxies.autoRecognize": false,
      interceptKnownFileTypes: false,
    })) {
      const expression = new RegExp(`("${key.replaceAll(".", "\\.")}": )true`);
      if (!expression.test(source)) throw new Error(`Preference drift: ${key}`);
      source = source.replace(expression, `$1${value}`);
    }
    change("'http://127.0.0.1:23119/'", "'http://127.0.0.1:23120/'");
  }
  if (path === "schema.js") change("/*ZOTERO_SCHEMA*/", JSON.stringify(schema));
  if (path === "itemSaver.js") {
    change("this._itemType = options.itemType;", "this._itemType = options.itemType;\n\tthis._translatorID = options.translatorID;\n\tthis._translatorVersion = options.translatorVersion;");
    change("uri: this._baseURI,", "uri: this._baseURI,\n\t\t\ttranslatorID: this._translatorID,\n\t\t\ttranslatorVersion: this._translatorVersion,");
    change("return this._saveToServer(items, attachmentCallback, itemsDoneCallback);", "throw e; // Research Assistant never falls back to Zotero cloud.");
    change("if (!attachment.title) attachment.title", "if (attachment.mimeType !== 'application/pdf' && attachment.snapshot !== false) return false;\n\t\t\t\tif (!attachment.title) attachment.title");
    change('await Zotero.Connector.callMethod("saveItems", payload)', 'const saved = await Zotero.Connector.callMethod("saveItems", payload);\n\t\tfor (const item of saved?.items || []) {\n\t\t\tfor (const warning of item.warnings || []) {\n\t\t\t\tif (warning.code === "unsupported_item_type") Zotero.Messaging.sendMessage("progressWindow.error", ["unsupportedItemType", warning.originalItemType, warning.storedItemType]);\n\t\t\t}\n\t\t}');
    change("if (!item.hasAttachmentResolvers) {", "if (!item.hasAttachmentResolvers) {\n\t\t\t\tif (!attachment) attachment = { id: item.id + '-no-pdf', parentItem: item.id, title: '未找到可访问 PDF（题录已保存）', mimeType: 'application/pdf' };");
  }
  if (path === "inject/pageSaving.js") {
    change("const onTranslatorFallback = (oldTranslator, newTranslator) => {", "let captureTranslator = translators[0];\n\t\tconst onTranslatorFallback = (oldTranslator, newTranslator) => {\n\t\t\tcaptureTranslator = newTranslator;");
    change("new Zotero.ItemSaver({ sessionID, itemType, baseURI: document.location.href, proxy })", "new Zotero.ItemSaver({ sessionID, itemType, baseURI: document.location.href, proxy, translatorID: captureTranslator.translatorID, translatorVersion: captureTranslator.lastUpdated })");
    source = source.replaceAll("snapshot: true", "snapshot: false");
    // No page snapshots or silent standalone cloud fallback, even when called programmatically.
    change("async _saveAsWebpage({ title, saveSnapshot } = {}) {", "async _saveAsWebpage({ title, saveSnapshot } = {}) {\n\t\tsaveSnapshot = false;");
    change("await Zotero.ItemSaver.saveAttachmentToServer(standaloneAttachment);", "throw e; // No Zotero cloud endpoint in this product.");
    change("// Clear session details on failure, so another save click tries again\n\t\t\tthis._clearSession();", "// Clear session details on failure, so another save click tries again\n\t\t\tthis._clearSession();\n\t\t\tif (e.name === 'Research Assistant Connector Error') {\n\t\t\t\tZotero.Messaging.sendMessage('progressWindow.done', [false, 'connectionError', e.message]);\n\t\t\t\treturn;\n\t\t\t}");
    const connectionCatch = "else if (!e.value || e.value.libraryEditable != false) {";
    if (source.split(connectionCatch).length !== 3) throw new Error("Webpage/standalone connection catch drift");
    source = source.replaceAll(connectionCatch, "else if (e.name === 'Research Assistant Connector Error') {\n\t\t\t\tZotero.Messaging.sendMessage('progressWindow.done', [false, 'connectionError', e.message]);\n\t\t\t}\n\t\t\telse if (!e.value || e.value.libraryEditable != false) {");
    const notes = source.match(/\n\t{4}if \(item.notes\) \{[\s\S]*?\n\t{4}\}/g);
    if (notes?.length !== 1) throw new Error("Upstream note progress drift");
    source = source.replace(notes[0], "");
  }
  if (path === "background.js") {
    change("_showNoteContextMenuItems(translators, saveMenuID);", "/* Notes are preserved in raw metadata, but note creation is not offered. */");
    change("fns.forEach((fn) => fn());", "fns.filter((_, index) => withSnapshot ? index === 1 : index === 0).forEach((fn) => fn());");
  }
  if (path === "ui/ProgressWindow.jsx") {
    change("{this.renderTargetSelector()}", "{/* The destination is the single Research Assistant library. */}");
    change("renderHeadline() {\n\t\treturn (", "renderHeadline() {\n\t\tconst connectionFailed = this.state.errors.some(([code]) => code === 'connectionError');\n\t\treturn (");
    change("{this.state.headlineText}", "{connectionFailed ? '连接失败' : this.state.headlineText}");
    change("{this.state.targets\n\t\t\t\t\t? this.renderHeadlineSelect()\n\t\t\t\t\t: (this.state.target ? this.renderHeadlineTarget() : \"\")}", "{!connectionFailed && this.state.target ? this.renderHeadlineTarget() : \"\"}");
    change('let targetName = target?.name || "zotero.org";', 'let targetName = target?.name || "";');
    change("addError() {\n\t\tthis.setState", "addError() {\n\t\tif (arguments[0] === 'connectionError') document.getElementById('messageAlert').textContent = '连接失败';\n\t\tthis.setState");
    change("{this.renderErrors()}", "{this.renderErrors()}\n\t\t\t\t{Array.from(this.state.itemProgress.values()).some(item => item.parentItem && item.failed) && <button className=\"research-assistant-retry\" onClick={() => this.sendMessage('updated', { target: this.state.target, note: '', tags: [], retryAttachments: true })}>重试 PDF</button>}");
    source = source.replaceAll("'Zotero'", "'Research Assistant'");
    change('if (err === "translationError") {', 'if (err === "connectionError") {\n\t\t\tcontents = typeof args[0] === "string" ? args[0] : "桌面连接无法确认，已停止传输。";\n\t\t}\n\t\telse if (err === "unsupportedItemType") {\n\t\t\tcontents = `类型 ${args[0]} 暂按 ${args[1]} 保存，原始题录字段已保留。`;\n\t\t}\n\t\telse if (err === "translationError") {');
    change("p.percentage = progress;", "p.percentage = progress;\n\t\t\t\tp.failed = false;");
  }
  if (path === "inject/progressWindow_inject.js") {
    change("resaveAttachments: !lastSuccessfulTarget.filesEditable && data.target.filesEditable,", "resaveAttachments: data.retryAttachments || (!lastSuccessfulTarget.filesEditable && data.target.filesEditable),");
  }
  if (path === "progressWindow/progressWindow.html") {
    change('<link rel="stylesheet" href="progressWindow.css" />', '<link rel="stylesheet" href="progressWindow.css" />\n\t\t<link rel="stylesheet" href="../research-assistant/progress.css" />');
  }
  if (path === "offscreen/offscreenSandbox.html") {
    // DOMParser checks captured style attributes against this sandbox policy.
    // Retain them for translators; URLs in CSS still hit default-src 'none'.
    change("default-src 'none'; script-src 'self' 'unsafe-eval';", "default-src 'none'; script-src 'self' 'unsafe-eval'; style-src 'unsafe-inline';");
    // frame-ancestors is ignored in meta policies. This packaged sandbox is not
    // web-accessible; retain the existing manifest sandbox and resource boundary.
    change("form-action 'none'; frame-ancestors 'self';", "form-action 'none';");
  }
  if (path.endsWith(".html")) {
    source = source.replace(/<!--BEGIN DEBUG-->[\s\S]*?<!--END DEBUG-->/g, "")
      .replace(/\s*<!-- SAFARI -->[\s\S]*?<!-- \/SAFARI -->/g, "");
  }
  return source;
}
