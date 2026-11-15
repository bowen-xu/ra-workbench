// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from "node:assert/strict";
import test from "node:test";
import { translatorLicense } from "../scripts/translator-licenses.mjs";

test("license inventory recognizes wrapped notices and preserves GPL licensing", () => {
  assert.equal(translatorLicense("/* GNU Affero General Public License, version 3 of\n the License, or any later version. */").license, "AGPL-3.0-or-later");
  assert.equal(translatorLicense("// GNU General Public License version 3; any later version.\n").license, "GPL-3.0-or-later");
  assert.equal(translatorLicense("/* GNU General Public License version 2; any later version. */").combinedVersion, "GPL-3.0");
  assert.equal(translatorLicense("/* Licensed under the Apache License, Version 2.0. */").license, "Apache-2.0");
  assert.equal(translatorLicense("/* MIT License */").license, "MIT");
});

test("unknown explicit licensing and GPLv2-only cannot silently use the project default", () => {
  assert.equal(translatorLicense("/* GNU General Public License version 2 only. */").compatible, false);
  assert.equal(translatorLicense("/* Licensed under a proprietary licence. */").compatible, false);
  assert.equal(translatorLicense("/* Redistribution and use in source and binary forms, subject to unreviewed clauses. */").compatible, false);
  assert.equal(translatorLicense("/* A Zotero translator without an individual license notice. */").basis, "Zotero-project-default");
});
