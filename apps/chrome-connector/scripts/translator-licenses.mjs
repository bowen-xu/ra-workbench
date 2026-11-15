// SPDX-License-Identifier: AGPL-3.0-or-later
// A source inventory, not a relabeling of individual translators' licenses.
export const projectLicensing = "https://www.zotero.org/support/licensing";
export function translatorLicense(source) {
  const comments = (source.split("/** BEGIN TEST CASES **/")[0]
    .match(/\/\*[\s\S]*?\*\/|\/\/[^\n]*/g) || []).join(" ").replace(/[\s*/]+/g, " ");
  if (comments.includes("GNU Affero General Public License")) {
    if (!/version 3/.test(comments)) return { license: "unresolved-AGPL", basis: "per-file-notice", compatible: false };
    return { license: /any later version/.test(comments) ? "AGPL-3.0-or-later" : "AGPL-3.0-only", basis: "per-file-notice", source: "https://www.gnu.org/licenses/agpl-3.0.html", compatible: true };
  }
  if (comments.includes("GNU General Public License")) {
    if (/version 3/.test(comments)) return { license: /any later version/.test(comments) ? "GPL-3.0-or-later" : "GPL-3.0-only", basis: "per-file-notice", source: "https://www.gnu.org/licenses/gpl-3.0.html", compatible: true };
    if (/version 2/.test(comments) && /any later version/.test(comments)) {
      return { license: "GPL-2.0-or-later", combinedVersion: "GPL-3.0", basis: "per-file-notice", source: "https://www.gnu.org/licenses/gpl-3.0.html#section13", compatible: true };
    }
    return { license: "unresolved-GPL", basis: "per-file-notice", compatible: false };
  }
  if (comments.includes("Apache License")) {
    return { license: "Apache-2.0", basis: "per-file-notice", source: "https://www.apache.org/licenses/LICENSE-2.0", compatible: true };
  }
  if (comments.includes("MIT License")) {
    return { license: "MIT", basis: "per-file-notice", compatible: true };
  }
  if (/Redistribution and use in source and binary forms/.test(comments)) {
    return { license: "BSD-family-review-required", basis: "per-file-notice", compatible: false };
  }
  if (/licensed|licence|creative commons|all rights reserved|public domain|free software/i.test(comments)) {
    return { license: "unresolved-explicit-notice", basis: "per-file-notice", compatible: false };
  }
  return { license: "AGPL-3.0-only", basis: "Zotero-project-default", source: projectLicensing, compatible: true };
}
