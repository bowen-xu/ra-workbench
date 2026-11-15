# Research Assistant Connector — source and licenses

This is an independent adaptation of Zotero Connector, not an official Zotero
product. Zotero is a trademark of the Corporation for Digital Scholarship.
Research Assistant's extension adapters and build scripts are provided under
AGPL-3.0-or-later. The upstream notices and full AGPL license in `COPYING` remain
in all reused source files.

Exact public repositories and revisions are recorded in `upstream-lock.json`.
The build materializes corresponding upstream and adaptation source in the
repository's `.tmp/chrome-connector-source/` directory. To rebuild, install the
locked workspace dependencies with `pnpm install --frozen-lockfile`, then run
`pnpm connector:build -- --fetch`. No unpublished binary translator dependency is
required. Keep the source directory, this source repository and license notices
with any copy supplied to another person; distribution must provide the complete
corresponding source under the applicable licenses.

Reused components:

- Zotero Connector, Translate and Utilities: AGPLv3 or later; source notices
  identify Center for History and New Media, George Mason University,
  Corporation for Digital Scholarship and individual contributors.
- The complete 750-file site-translator snapshot retains all source notices:
  637 explicit AGPLv3 notices, 35 GPLv3-or-later notices, one GPLv2-or-later
  notice (combined using its GPLv3 option), one Apache-2.0 notice, and 76 files
  using Zotero's project-wide AGPLv3 default. Per-file provenance, hashes and
  original licenses are in `translators/licenses.json`. GPL-only source remains
  GPL licensed; it is not relabeled AGPL. Combining the GPLv3 and AGPLv3 parts
  follows [GPLv3 section 13](https://www.gnu.org/licenses/gpl-3.0.html#section13)
  and the corresponding section in AGPLv3. `licenses/GPL-3.0.txt` retains the
  official GNU text. Intellixir's Apache notice remains in its original source;
  the Apache terms are also preserved by the bundled DOMPurify license.
- Zotero schema: the public pinned schema is embedded by the upstream build
  contract. Its checkout has no standalone license declaration. The licensing
  basis is Zotero's [official project-wide licensing statement](https://www.zotero.org/support/licensing),
  which applies AGPLv3 unless otherwise indicated. The exact schema source is
  preserved alongside `COPYING`; it is not represented as CC0.
- React, React DOM, React DOM Factories and PropTypes: MIT; retain their bundled
  source notices and package licenses in the corresponding source bundle.
- DOMPurify: Apache-2.0 or MPL-2.0, retained source notice. Its package source and
  license are included in `.tmp/chrome-connector-source/dependencies/`; bundled
  dependency licenses are also included in the loadable extension's `licenses/`.

SingleFile, Google Docs integration and Zotero account code are not used. App
toolbar/category artwork is generated from original Research Assistant code;
Zotero app marks are replaced. No browser cookies or authorization headers are
sent to the Research Assistant desktop bridge.
