// SPDX-License-Identifier: AGPL-3.0-or-later
// Replaces the unrelated upstream Zotero account/cloud and Google Docs ports.
(() => {
  const unsupported = async () => { throw new Error("Zotero cloud saving and account integration are disabled in Research Assistant Connector"); };
  Zotero.API = {
    authorize: unsupported, onAuthorizationComplete: unsupported,
    getUserInfo: async () => null, clearCredentials: async () => undefined,
    createItem: unsupported, uploadAttachment: unsupported, run: unsupported,
  };
  Zotero.GoogleDocs = { API: {} };
  Zotero.GoogleDocs_API = { onAuthComplete: unsupported, run: unsupported };
})();
