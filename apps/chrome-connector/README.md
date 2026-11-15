# Research Assistant Connector

An independently branded Chrome MV3 adaptation of Zotero Connector. It uses the
actual pinned Zotero detection, offscreen translator sandbox, item selector,
page-save sessions, browser PDF retrieval and progress window. It sends metadata
and PDF bytes to the automatically discovered Research Assistant desktop receiver.

## Interactive preview

```sh
pnpm connector:preview
```

Open `http://127.0.0.1:5187/`. This rebuilds the extension and serves its actual
options page, upstream item selector and React progress window in an outer
preview frame. Their markup, scripts and CSS are the same production files;
the preview injects only mock Chrome APIs, messages and desktop responses.
No extension installation or running desktop application is needed.

The preview starts with a successful single-paper capture. Switch to PDF
downloading, failed PDF (use the real **重试 PDF** button), desktop offline,
another application occupying the port, or an incompatible receiver protocol.
**模拟采集** replays the mock download. **多篇文献** opens the actual selector;
confirming selected papers shows their progress. **插件设置** provides an in-memory
receiver for testing automatic connection detection and its actual options controls.
Connection detection runs automatically. Each view reload/reset creates fresh
mock state; it never reads real browser storage or desktop data.

Use `RA_CONNECTOR_PREVIEW_PORT` for a different port. The server only binds
loopback; browser fetches cannot reach native services or external websites.
Preview adapters are not included in the unpacked extension. This previews UI
behavior, not real extraction, downloading or native persistence.

## Development

From the repository root:

```sh
pnpm install --frozen-lockfile
pnpm connector:build -- --fetch
pnpm connector:check
pnpm --filter @research-assistant/chrome-connector test:build
```

`--fetch` downloads missing exact source revisions from `upstream-lock.json`.
With the reference caches populated, omit it for an offline build. All sources
are read through `git archive` at locked commits, so worktree changes or a moving
upstream branch cannot alter the build. The source caches are not modified.
Build output is `.tmp/chrome-connector`; complete corresponding source is
`.tmp/chrome-connector-source`, including adapter source and runtime dependency
source. No installer, release artifact or store package is created.

Open `chrome://extensions`, enable Developer mode, choose **Load unpacked** and
select `.tmp/chrome-connector`. Open the desktop application; the extension
automatically detects it without a pairing code, terminal command or separate
native helper. Formal desktop is the default; select development explicitly
in extension options when needed. The extension uses
`127.0.0.1:23120` for the formal app and `127.0.0.1:23121` for the development app.
It requires Chrome 116 or later. If the port belongs to another application,
the extension stops after an empty discovery request and reports the connection error.
It does not try arbitrary ports or send literature/PDF data to that application.

Open a paper page and click the extension button. Multiple-result pages use the
original Zotero item selector. Detected item metadata is saved first, then the
browser attempts accessible PDFs. A failed PDF keeps the item and exposes
**重试 PDF** in the progress window; its existing session and attachment identities
are reused. Metadata failures are reported in the current capture progress;
after restoring the connection, return to the source page and capture again.
Options show connection status and detection, with no metadata retry queue or
receiver-reset controls. Requests retain caller-provided session/item identities
for native receipt deduplication. A receiver binding cannot silently move to
another desktop library. Browser storage contains connection settings, not a
persistent metadata request queue or another authoritative library.

## Scope

- The complete pinned bundle includes 750 translators, including arXiv, PubMed,
  DOI, Crossref REST, Embedded Metadata, COinS, RIS, BibTeX and ScienceDirect.
  `translators/licenses.json` records each source filename, identity, SHA-256 and
  licensing basis. Individual GPL/Apache notices are retained; no-notice Zotero
  source uses the official project's AGPLv3 default. Coverage still depends on
  live sites and the supported desktop capabilities.
- Uses browser cookies/referrer for browser-side retrieval. Cookies and
  authorization headers are removed before desktop transport. Each operation
  first checks the application's protocol, environment and receiver identity;
  its short-lived connection token stays in the background runtime and is
  excluded from storage, content-script responses and upstream Prefs RPCs.
- Supports accessible PDFs up to 64 MiB, generic webpage metadata and direct PDF
  saving. Desktop recognition and OA resolvers report unsupported capability.
- No Zotero cloud/account fallback, active-URL reporting, remote translator
  updates, Google Docs, snapshots, EPUB, target switching, tag editing or note
  creation controls. Original untranslated metadata, including unsupported item
  fields, remains the desktop adapter's provenance record.
- Failed PDF uploads can be retried while their source page/session is open.
  Metadata requests and PDF bodies are not persisted as an independent browser
  queue or attachment store. Closing/reloading the original tab
  requires another explicit capture. Complete stock Zotero behavior or site
  coverage is not asserted.

## Validation

`test` exercises the local transport without network or upstream caches;
`test:build` requires a built extension and checks actual migrated source,
notices and no-cloud failure behavior. `test:browser` launches an isolated
Chromium profile, loads the production MV3 tree, runs real offscreen translator
detection on a synthetic scholarly page and verifies item plus browser PDF
transfer. Its synthetic servers and browser must run together outside the
sandbox under the repository's local-server debugging rule.
`test:preview` checks that preview pages and styles reuse the actual build,
that mock transport stays in memory, and that preview adapters do not enter
the loadable extension. It requires the extension to have been built.
`test:discovery` uses real loopback HTTP fixtures to check that unrelated
software receives no metadata or PDF upload, including when it replaces a
previously valid receiver. It asserts the fixed development URL before mapping
that address to an isolated ephemeral fixture port. Run it outside the sandbox.

For browser-enforced sandbox CSP regression, build first, then run
`node apps/chrome-connector/scripts/csp-fixture-server.mjs` from the repository
root outside the sandbox. Open its printed loopback URL in the in-app browser.
The fixture runs the actual built capture and HTTP document parsers under the
production sandbox policy delivered by the same meta element, mocking only
lifecycle/HTTP wrapper ports. It does not substitute an HTTP CSP header. The
original-policy link reproduces inline-style errors; the fixed policy preserves
metadata and inline CSS while direct fetch, images, external CSS and inline
scripts remain blocked. `/network-count` must report no forbidden requests.
This is a policy/parser test, not a full MV3 extension or publisher-site test.

```sh
pnpm --filter @research-assistant/chrome-connector test:browser
```

Set `RA_CONNECTOR_EXTENDED_SMOKE=1` for the real COinS two-candidate selection UI,
`RA_CONNECTOR_RETRY_SMOKE=1` for a first-download 403 followed by the visible PDF
retry button, or `RA_CONNECTOR_REAL_SITE_SMOKE=1` for isolated public arXiv,
PubMed and DOI-publisher attempts. These use a synthetic receiver on port 23120;
they require that port to be unused and never connect to personal app data.

For a disposable native development application with isolated data, the same
script supports `RA_CONNECTOR_NATIVE_ENDPOINT=http://127.0.0.1:23121/`.
It performs the same automatic discovery as the extension; no credential file
or manual code is required. This mode uses the native receiver instead of the
synthetic bridge.
Real publisher, institutional-access, multi-selection and restart acceptance
must be recorded separately from the synthetic fixture result.

## Adaptation map

| Source | Adaptation |
| --- | --- |
| Pinned `gulpfile.js` script lists | Original content/background ordering; omit unrelated Google Docs/updater imports. |
| `zotero.js`, `api.js`, `zotero_config.js` | Research Assistant defaults/branding; protected separate storage; disabled account/cloud integrations. |
| `repo.js` ports | Load a reproducible local translator bundle through the original cache and priority machinery. |
| `itemSaver.js`, `inject/pageSaving.js` | Original save/download flow; include exact translator provenance; PDF-only files, visible unsupported-type warnings, no cloud fallback. |
| `ui/ProgressWindow.jsx`, `progressWindow_inject.js` | Fixed global library, remove target/tags/notes controls, reuse original attachment retry and progress messaging. |
| `connector.js` port | Loopback transport with strict per-operation discovery, launch-bound credentials and original Zotero v3 request shapes; native receipts deduplicate retries. |
| Manifest/resources | Chrome MV3 sandbox retained; original code-drawn document icons; no SingleFile or style interception rules. |

All edits to upstream output are explicit, drift-checked replacements in
`scripts/patch-upstream.mjs`. See [NOTICE.md](NOTICE.md) and [COPYING](COPYING)
for licensing and corresponding-source obligations.
