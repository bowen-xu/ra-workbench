import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { checkNativeBoundaries } from "./native-workbench-boundaries.mjs";

const base = () => ({
  roots: { ai: { owner: "core" }, storage: { owner: "core" }, writing: { owner: "plugin/writing" }, literature: { owner: "plugin/literature" } },
  physicalOwners: [{ prefix: "core/", owner: "core" }, { prefix: "plugins/writing/", owner: "plugin/writing" }],
  entries: [
    { path: "crate::ai::EmbeddingAccess", roles: ["policy", "repository"] },
    { path: "crate::writing::agent_compile_synctex_source", roles: ["app"] },
    { path: "crate::storage::AppStore", roles: ["app", "repository"] },
    { path: "crate::ai::JsonCredentialStore", roles: ["app"] },
    { path: "crate::writing::CitationReader", roles: ["repository", "app"] },
  ], consumers: [], facades: [], pending: [], coverage: "Finite fixtures; not general Rust semantics",
});
const config = (file, owner = "core", role = "policy", module = "crate::ai") => ({ file, owner, role, module });
function check(files, policy, exceptions = []) {
  const root = mkdtempSync(join(tmpdir(), "native-boundaries-"));
  try {
    for (const [file, text] of Object.entries(files)) {
      mkdirSync(dirname(join(root, file)), { recursive: true });
      writeFileSync(join(root, file), text);
    }
    return checkNativeBoundaries(root, policy, exceptions);
  } finally { rmSync(root, { recursive: true, force: true }); }
}
function one(text, owner = "core", role = "policy") {
  const policy = base(); policy.consumers = [config("core/consumer.rs", owner, role)];
  return check({ "core/consumer.rs": text }, policy);
}

test("01 same-owner implementation import passes", () => {
  assert.equal(one("use crate::ai::private::implementation;").ok, true);
});
test("02 registered Core capability accepts local as alias", () => {
  const result = one("use crate::ai::{EmbeddingAccess as Embed}; fn run(_: Embed) {}", "plugin/literature");
  assert.equal(result.ok, true);
  assert.equal(result.checkedReferences, 1);
});
test("03 App may select registered composition-only factory", () => {
  assert.equal(one("use crate::writing::agent_compile_synctex_source as factory;", "app", "app").ok, true);
});
test("04 internal repository permission does not transfer to ordinary policy", () => {
  const text = "use crate::writing::CitationReader;";
  assert.equal(one(text, "plugin/literature", "repository").ok, true);
  const result = one(text, "plugin/literature");
  assert.equal(result.ok, false);
  assert.equal(result.violations[0].reason, "consumer-role-denied");
});
test("05 Core direct qualified private dependency is rejected", () => {
  const result = one("fn run() { crate::writing::private_compile(); }");
  assert.equal(result.ok, false);
  assert.equal(result.violations[0].reason, "core-reverse-implementation");
});
test("06 module alias cannot hide reverse implementation dependency", () => {
  const result = one("use crate::writing as domain; fn run() { domain::private_compile(); }");
  assert.equal(result.ok, false);
  assert.equal(result.violations.length, 2);
  assert.equal(result.violations[1].target, "crate::writing::private_compile");
});
test("07 grouped use with as cannot legalize private Core implementation", () => {
  const result = one("use crate::ai::{embedding::{PrivateProvider as Provider}};", "plugin/literature");
  assert.equal(result.ok, false);
  assert.equal(result.violations[0].target, "crate::ai::embedding::PrivateProvider");
});
test("08 facade rejects new private reexport", () => {
  const policy = base(); policy.consumers = [{ ...config("core/facade.rs"), mode: "facade" }];
  policy.facades = [{ file: "core/facade.rs", exports: [] }];
  const result = check({ "core/facade.rs": "pub(crate) use crate::ai::private::Provider as NewProvider;" }, policy);
  assert.equal(result.ok, false);
  assert.equal(result.violations[0].reason, "public-export-widened-or-origin-changed");
  const glob = check({ "core/facade.rs": "pub(crate) use crate::ai::private::*;" }, policy);
  assert.equal(glob.ok, false);
  assert.match(glob.unsupported[0].reason, /^Public glob reexport/);
  assert.equal(check({ "core/facade.rs": "use crate::ai::private::*;" }, policy).ok, true);
});
test("09 simple path type alias is checked at its private origin", () => {
  const result = one("type Private = crate::ai::embedding::PrivateProvider;", "plugin/literature");
  assert.equal(result.ok, false);
  assert.equal(result.violations[0].target, "crate::ai::embedding::PrivateProvider");
});
test("10 raw store and credential owner remain denied through facade names", () => {
  const result = one("use crate::{storage::AppStore as Store, ai::JsonCredentialStore as Credentials};", "plugin/literature");
  assert.equal(result.ok, false);
  assert.equal(result.violations.length, 2);
  assert.ok(result.violations.every(item => item.reason === "consumer-role-denied"));
});
test("11 literal path cannot reload another owner's implementation", () => {
  const policy = base(); policy.consumers = [config("core/consumer.rs")];
  const result = check({ "core/consumer.rs": '#[path = "../plugins/writing/private.rs"] mod local;', "plugins/writing/private.rs": "fn run() {}" }, policy);
  assert.equal(result.ok, false);
  assert.equal(result.violations[0].reason, "cross-owner-module-path");
});
test("12 App root literal path remains legal composition", () => {
  const policy = base(); policy.consumers = [config("lib.rs", "app", "app", "crate")];
  assert.equal(check({ "lib.rs": '#[path = "plugins/writing/private.rs"] mod writing;', "plugins/writing/private.rs": "fn run() {}" }, policy).ok, true);
});
test("13 changed named export origin or private module visibility fails", () => {
  const policy = base(); policy.consumers = [config("core/facade.rs")];
  policy.facades = [{ file: "core/facade.rs", exports: [{ name: "EmbeddingAccess", target: "crate::ai::embedding::EmbeddingAccess", visibility: "pub(crate)" }], modules: [{ name: "embedding", visibility: "private" }] }];
  for (const text of ["mod embedding; pub(crate) use alternate::EmbeddingAccess;", "pub(crate) mod embedding; pub(crate) use embedding::EmbeddingAccess;"]) {
    const result = check({ "core/facade.rs": text }, policy);
    assert.equal(result.ok, false);
    assert.ok(result.stale.length || result.violations.length);
  }
});
test("14 resolved exact exception is stale", () => {
  const policy = base(); policy.consumers = [config("core/consumer.rs")];
  const exception = { from: "core/consumer.rs", target: "crate::writing::private_compile", kind: "path", count: 1, reason: "existing fixed migration" };
  const result = check({ "core/consumer.rs": "fn run() {}" }, policy, [exception]);
  assert.equal(result.ok, false);
  assert.equal(result.stale[0].reason, "exception-count-changed");
});
test("15 exception count growth and new location fail", () => {
  const policy = base(); policy.consumers = [config("core/consumer.rs"), config("core/other.rs")];
  const exception = { from: "core/consumer.rs", target: "crate::writing::private_compile", kind: "path", count: 1, reason: "existing fixed migration" };
  const call = "fn run() { crate::writing::private_compile(); }";
  assert.equal(check({ "core/consumer.rs": call, "core/other.rs": "" }, policy, [exception]).ok, true);
  for (const files of [{ "core/consumer.rs": call + call, "core/other.rs": "" }, { "core/consumer.rs": call, "core/other.rs": call }]) {
    const result = check(files, policy, [exception]);
    assert.equal(result.ok, false);
    assert.ok(result.violations.length);
  }
});
test("16 protected glob, nonliteral and cfg_attr paths fail explicitly", () => {
  for (const text of ["use crate::writing::*;", '#[path = concat!("../", "plugins/writing/private.rs")] mod local;', '#[cfg_attr(unix, path = "../plugins/writing/private.rs")] mod local;']) {
    const result = one(text);
    assert.equal(result.ok, false, text);
    assert.ok(result.unsupported.length, text);
  }
});
test("17 registered crate-visible test helpers require the exact cfg and visibility", () => {
  const policy = base();
  policy.consumers = [{...config("core/consumer.rs"), testItems: [
    {kind: "fn", name: "fixture", cfg: "test", visibility: "pub(crate)"},
  ]}];
  const body = "fn fixture() { crate::writing::private_compile(); }";
  assert.equal(check({"core/consumer.rs": "#[cfg(test)] pub(crate) " + body}, policy).ok, true);
  for (const prefix of ["#[cfg(unix)] pub(crate) ", "#[cfg(test)] pub ", "#[cfg(test)] "]) {
    const result = check({"core/consumer.rs": prefix + body}, policy);
    assert.equal(result.ok, false);
    assert.ok(result.unsupported.length);
  }
});
