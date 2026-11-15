// SPDX-License-Identifier: AGPL-3.0-or-later
// Installed before upstream background.js calls Zotero.initGlobal().
(() => {
  const trustedStorage = chrome.storage.local.setAccessLevel({ accessLevel: "TRUSTED_CONTEXTS" });
  const request = ResearchAssistant.createTransport(chrome.storage.local, fetch, Zotero.version);
  Zotero.Connector.CommunicationError = ResearchAssistant.CommunicationError;
  Zotero.Connector.callMethod = async (options, data) => {
    await trustedStorage;
    try {
      const response = await request(options, data);
      Zotero.Connector.isOnline = true;
      Zotero.Connector.clientVersion = "9.0.0"; // Upstream UI protocol compatibility, not app version.
      return response;
    } catch (error) {
      Zotero.Connector.isOnline = false;
      throw error;
    }
  };
  Zotero.Connector.checkIsOnline = async () => {
    try { await Zotero.Connector.ping(); return true; } catch { return false; }
  };
  Zotero.Connector.reportActiveURL = () => undefined;
  Zotero.Connector.prefs = {
    reportActiveURL: false, automaticSnapshots: false, downloadAssociatedFiles: true,
    canUserAddNote: false, supportsTagsAutocomplete: false,
  };
  const originalPreferences = Zotero.Connector._processPreferences.bind(Zotero.Connector);
  Zotero.Connector._processPreferences = prefs => originalPreferences({
    ...prefs, reportActiveURL: false, automaticSnapshots: false,
    canUserAddNote: false, supportsTagsAutocomplete: false,
  });
  const originalPrefsInit = Zotero.Prefs.init.bind(Zotero.Prefs);
  Zotero.Prefs.init = async () => {
    await trustedStorage;
    await originalPrefsInit();
    // Upstream getAll() is available to content scripts and the translator sandbox.
    delete Zotero.Prefs.syncStorage[ResearchAssistant.STORAGE_KEY];
  };
  const bundledMetadata = async () => (await fetch(chrome.runtime.getURL("translators/metadata.json"))).json() as Promise<unknown>;
  Zotero.Repo.getTranslatorMetadataFromZotero = bundledMetadata;
  Zotero.Repo.getTranslatorMetadataFromServer = bundledMetadata;
  Zotero.Repo.getTranslatorCode = async id => {
    if (!/^[a-f0-9-]{36}$/.test(id)) throw new Error("Invalid translator ID");
    const response = await fetch(chrome.runtime.getURL(`translators/${id}.js`));
    if (!response.ok) throw new Error("Translator is not in the pinned bundle");
    return response.text();
  };
  Zotero.Translators.keepTranslatorsUpdated = async () => undefined;
  Zotero.ContentTypeHandler.init = () => undefined;
  Zotero.ContentTypeHandler.enable = () => undefined;
  Zotero.ContentTypeHandler.disable = () => undefined;
  Zotero.Connector_Debug.submitReport = async () => { throw new Error("Remote diagnostic reporting is disabled"); };
})();
