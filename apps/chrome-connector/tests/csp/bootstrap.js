// SPDX-License-Identifier: AGPL-3.0-or-later
// Only platform/lifecycle ports are mocked; parsing uses the built Zotero handlers.
const cspMessages = new Map();
window.Zotero = {
  isChromium: true,
  isDebug: false,
  Translate: { Web: class { setDocument(document) { this.document = document; } }, ItemSaver: class {} },
  HTTP: { wrapDocument: document => document, VALID_DOM_PARSER_CONTENT_TYPES: new Set(["text/html"]), determineDOMParserContentType: type => type },
  OffscreenSandbox: { addMessageListener: (name, handler) => cspMessages.set(name, handler) },
};
const cspViolations = [];
document.addEventListener("securitypolicyviolation", event => {
  cspViolations.push({ directive: event.effectiveDirective, blocked: event.blockedURI });
});
