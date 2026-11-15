import {logicalProjectPath} from "./project-layout.mjs";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { dirname, relative, resolve, sep } from "node:path";

const posix = value => value.split(sep).join("/");
const word = value => /^[A-Za-z_][A-Za-z0-9_]*$/.test(value ?? "");
const sameOrChild = (value, prefix) => value === prefix || value.startsWith(prefix + "::");

// Deliberately lexical: no expansion, type inference or general Rust name resolver.
// Offsets survive comments/strings, including raw strings and nested block comments.
export function rustTokens(source) {
  const tokens = [];
  let index = 0, line = 1;
  const push = (value, start, startLine, kind = "punct") => tokens.push({ value, start, line: startLine, kind });
  while (index < source.length) {
    const start = index, startLine = line, rest = source.slice(index);
    const advance = size => { line += (source.slice(index, index + size).match(/\n/g) ?? []).length; index += size; };
    if (/^\s/.test(rest)) { advance(1); continue; }
    if (rest.startsWith("//")) { advance(rest.indexOf("\n") < 0 ? rest.length : rest.indexOf("\n")); continue; }
    if (rest.startsWith("/*")) {
      let depth = 1, end = index + 2;
      while (end < source.length && depth) {
        if (source.startsWith("/*", end)) { depth++; end += 2; }
        else if (source.startsWith("*/", end)) { depth--; end += 2; }
        else end++;
      }
      if (depth) throw new Error(`Unclosed block comment at ${startLine}`);
      advance(end - index); continue;
    }
    const raw = rest.match(/^(?:b|c)?r(#+)?"/);
    if (raw) {
      const ending = '"' + (raw[1] ?? ""), end = source.indexOf(ending, index + raw[0].length);
      if (end < 0) throw new Error(`Unclosed raw string at ${startLine}`);
      push(source.slice(index + raw[0].length, end), start, startLine, "string");
      advance(end + ending.length - index); continue;
    }
    const quoted = rest.match(/^(?:b|c)?"(?:\\[\s\S]|[^"\\])*"|^(?:b)?'(?:\\(?:u\{[^}]+\}|x[\da-fA-F]{2}|[\s\S])|[^'\\\n])'/);
    if (quoted) {
      push(quoted[0], start, startLine, "string"); advance(quoted[0].length); continue;
    }
    const identifier = rest.match(/^(?:r#)?[A-Za-z_][A-Za-z0-9_]*/);
    if (identifier) { push(identifier[0].replace(/^r#/, ""), start, startLine, "ident"); advance(identifier[0].length); continue; }
    if (rest.startsWith("::")) { push("::", start, startLine); advance(2); continue; }
    push(source[index], start, startLine); advance(1);
  }
  return tokens;
}

function closing(tokens, at) {
  const end = { "{": "}", "[": "]", "(": ")" }[tokens[at]?.value];
  let depth = 1;
  for (let i = at + 1; i < tokens.length; i++) {
    if (tokens[i].value === tokens[at].value) depth++;
    if (tokens[i].value === end && --depth === 0) return i;
  }
  throw new Error(`Unclosed ${tokens[at]?.value} at ${tokens[at]?.line}`);
}

function useLeaves(tokens) {
  let at = 0;
  const leaves = [];
  function tree(prefix) {
    if (tokens[at]?.value === "{") {
      at++;
      while (at < tokens.length && tokens[at].value !== "}") {
        if (tokens[at].value === ",") { at++; continue; }
        tree(prefix);
      }
      if (tokens[at++]?.value !== "}") throw new Error("Unclosed use tree");
      return;
    }
    const name = tokens[at++]?.value;
    if (name === "*") { leaves.push({ path: [...prefix, "*"], alias: "*" }); return; }
    if (!word(name)) throw new Error("Unsupported use tree");
    const path = name === "self" && prefix.length ? prefix : [...prefix, name];
    if (tokens[at]?.value === "::") { at++; tree(path); return; }
    let alias = path.at(-1);
    if (tokens[at]?.value === "as") { at++; alias = tokens[at++]?.value; }
    if (!word(alias) && alias !== "_") throw new Error("Unsupported use alias");
    leaves.push({ path, alias });
  }
  tree([]);
  if (at !== tokens.length) throw new Error("Unsupported use suffix");
  return leaves;
}

function normalize(parts, module, aliases, roots) {
  if (aliases.has(parts[0])) return [...aliases.get(parts[0]).split("::"), ...parts.slice(1)].join("::");
  if (parts[0] === "crate") return parts.join("::");
  const base = module.split("::");
  if (parts[0] === "self") return [...base, ...parts.slice(1)].join("::");
  if (parts[0] === "super") {
    let count = 0;
    while (parts[count] === "super") count++;
    return [...base.slice(0, -count), ...parts.slice(count)].join("::");
  }
  return roots[parts[0]] ? ["crate", ...parts].join("::") : [...base, ...parts].join("::");
}

function visibility(tokens, at) {
  let previous = at - 1;
  if (tokens[previous]?.value === ")") {
    while (previous >= 0 && tokens[previous].value !== "(") previous--;
    if (tokens[previous - 1]?.value === "pub") return tokens.slice(previous - 1, at).map(t => t.value).join("");
  }
  return tokens[previous]?.value === "pub" ? "pub" : "private";
}

export function collectNativeReferences(text, config, roots) {
  const tokens = rustTokens(text), edges = [], exports = [], modules = [], unsupported = [], aliases = new Map();
  const covered = new Set(), testRanges = [];
  for (let i = 0; i < tokens.length; i++) {
    const expected = (config.testItems ?? []).find(item => item.kind === tokens[i].value && item.name === tokens[i + 1]?.value);
    if (!expected) continue;
    const before = tokens.slice(Math.max(0, i - 40), i).map(t => t.value).join("");
    if (!before.endsWith("#[cfg(" + expected.cfg + ")]" + (expected.visibility ?? ""))) {
      unsupported.push({ line: tokens[i].line, reason: "Registered test item " + expected.name + " changed cfg or visibility" }); continue;
    }
    let body = i + 2;
    while (body < tokens.length && !["{", ";"].includes(tokens[body].value)) body++;
    if (tokens[body]?.value === ";" && expected.kind === "mod") continue;
    if (tokens[body]?.value !== "{") { unsupported.push({ line: tokens[i].line, reason: "Registered test body is unavailable" }); continue; }
    testRanges.push([i, closing(tokens, body)]);
  }
  for (let i = 0; i < tokens.length; i++)
    if (tokens[i].kind === "ident" && tokens[i].value === "mod" && word(tokens[i + 1]?.value) && !testRanges.some(([start, end]) => i >= start && i <= end))
      aliases.set(tokens[i + 1].value, config.module + "::" + tokens[i + 1].value);
  const inTest = i => testRanges.some(([start, end]) => i >= start && i <= end);
  const add = (path, kind, line, alias) => edges.push({ from: config.file, target: path, kind, line, alias, role: config.role });
  for (let i = 0; i < tokens.length; i++) {
    if (inTest(i)) continue;
    const token = tokens[i];
    if (token.kind === "ident" && token.value === "use") {
      let end = i + 1;
      while (end < tokens.length && tokens[end].value !== ";") end++;
      try {
        for (const leaf of useLeaves(tokens.slice(i + 1, end))) {
          const target = normalize(leaf.path, config.module, aliases, roots);
          if (leaf.alias === "*") unsupported.push({
            line: token.line,
            reason: visibility(tokens, i) === "private"
              ? `Glob import is not supported: ${target}`
              : `Public glob reexport is not supported: ${target}`,
          });
          else {
            add(target, visibility(tokens, i) === "private" ? "use" : "pub-use", token.line, leaf.alias);
            aliases.set(leaf.alias, target);
            if (visibility(tokens, i) !== "private") exports.push({ name: leaf.alias, target, visibility: visibility(tokens, i) });
          }
        }
      } catch (error) { unsupported.push({ line: token.line, reason: error.message }); }
      for (let j = i; j <= end; j++) covered.add(j);
      i = end;
    } else if (token.kind === "ident" && token.value === "mod" && word(tokens[i + 1]?.value)) {
      const before = tokens.slice(Math.max(0, i - 35), i);
      let path;
      for (let j = before.length - 1; j >= 0; j--) {
        if (before[j].value === "path" && before[j + 1]?.value === "=") {
          path = before[j + 2]?.kind === "string" && before[j + 3]?.value === "]"
            ? before[j + 2].value.replace(/^"|"$/g, "") : null;
          break;
        }
        if ([";", "{" , "}"].includes(before[j].value)) break;
      }
      const attrs = before.map(t => t.value).join("");
      if (attrs.includes("cfg_attr") && attrs.includes("path")) unsupported.push({ line: token.line, reason: "cfg_attr module path is unsupported" });
      if (path === null) unsupported.push({ line: token.line, reason: "Nonliteral module path is unsupported" });
      modules.push({ name: tokens[i + 1].value, path, visibility: visibility(tokens, i), line: token.line });
    }
  }
  // Named import declarations carry their original route even if a local name is
  // shadowed later. Explicit aliases do not make an illegal introduction legal.
  for (let i = 0; i < tokens.length; i++) {
    if (covered.has(i) || inTest(i) || tokens[i].kind !== "ident") continue;
    if (tokens[i - 1]?.value === "::") continue;
    const parts = [tokens[i].value]; let end = i;
    while (tokens[end + 1]?.value === "::" && word(tokens[end + 2]?.value)) { parts.push(tokens[end + 2].value); end += 2; }
    if (parts.length < 2) continue;
    if (!["crate", "self", "super"].includes(parts[0]) && !aliases.has(parts[0]) && !roots[parts[0]]) continue;
    const target = normalize(parts, config.module, aliases, roots);
    add(target, "path", tokens[i].line);
    i = end;
  }
  return { edges, exports, modules, unsupported, testScopes: testRanges.length };
}

export function nativeBoundaryKey(edge) { return JSON.stringify([edge.from, edge.target, edge.kind]); }

export function checkNativeBoundaries(root, policy, exceptions = [], read = file => readFileSync(resolve(root, file), "utf8")) {
  root = realpathSync(root);
  const violations = [], unsupported = [], stale = [], edges = [], scans = new Map();
  const ownerOf = path => policy.roots[path.split("::")[1]]?.owner;
  const seenEntries = new Set();
  for (const entry of policy.entries) {
    if (seenEntries.has(entry.path) || !entry.path.startsWith("crate::") || !Array.isArray(entry.roles))
      stale.push({ key: entry.path, reason: "invalid-or-duplicate-public-entry" });
    seenEntries.add(entry.path);
  }
  const entryFor = target => policy.entries.filter(entry => entry.namespace ? target === entry.path : sameOrChild(target, entry.path)).sort((a, b) => b.path.length - a.path.length)[0];
  for (const config of policy.consumers) {
    const file = resolve(root, config.file);
    const target = existsSync(file) ? realpathSync(file) : file;
    const relativeFile = logicalProjectPath(root, target);
    if (relativeFile.startsWith("../") || relativeFile === ".." || (policy.sourcePrefix && !relativeFile.startsWith(policy.sourcePrefix))) {
      unsupported.push({ from: config.file, reason: "Consumer is outside the registered native source root" }); continue;
    }
    let scan;
    try { scan = collectNativeReferences(read(config.file), config, policy.roots); }
    catch (error) { unsupported.push({ from: config.file, reason: error.message }); continue; }
    if (config.mode === "facade") {
      scan.edges = scan.edges.filter(edge => edge.kind === "pub-use");
      scan.unsupported = scan.unsupported.filter(item => !item.reason.startsWith("Glob import"));
    }
    scans.set(config.file, scan); edges.push(...scan.edges);
    for (const item of scan.unsupported) {
      const glob = item.reason.match(/^Glob import is not supported: (.+)::\*$/)?.[1];
      const globEntry = glob && entryFor(glob);
      if (glob && (ownerOf(glob) === config.owner || (config.role === "app" && ownerOf(glob) === "core") || (globEntry?.allowGlob && globEntry.roles.includes(config.role)))) continue;
      unsupported.push({ from: config.file, ...item });
    }
    for (const edge of scan.edges) {
      const targetOwner = ownerOf(edge.target);
      if (!targetOwner || targetOwner === config.owner) continue;
      const entry = entryFor(edge.target);
      let reason;
      if (config.owner === "core" && targetOwner.startsWith("plugin") && !entry?.contract) reason = "core-reverse-implementation";
      else if ((config.role === "app" && targetOwner === "core") || config.role === "test") continue;
      else if (!entry) reason = "private-cross-owner-reference";
      else if (!entry.roles.includes(config.role)) reason = "consumer-role-denied";
      if (reason) violations.push({ ...edge, reason });
    }
    if (config.moduleDeclarations) {
      const signature = item => JSON.stringify([item.name, item.path ?? null, item.visibility]);
      const expected = new Set(config.moduleDeclarations.map(signature));
      const actual = new Set(scan.modules.map(signature));
      for (const value of actual) if (!expected.has(value)) violations.push({ from: config.file, target: value, kind: "module-contract", reason: "unregistered-module-declaration" });
      for (const value of expected) if (!actual.has(value)) stale.push({ key: config.file + ":" + value, reason: "module-declaration-removed-or-changed" });
    }
    for (const item of scan.modules) {
      if (!item.path) continue;
      const absolute = resolve(dirname(resolve(root, config.file)), item.path);
      const resolved = existsSync(absolute) ? realpathSync(absolute) : absolute;
      const file = logicalProjectPath(root, resolved);
      const targetOwner = policy.physicalOwners.find(item => file.startsWith(item.prefix))?.owner;
      if (!existsSync(absolute)) violations.push({ from: config.file, target: file, kind: "module-path", line: item.line, reason: "missing-module-path" });
      else if (targetOwner && targetOwner !== config.owner && config.role !== "app" && config.role !== "test") violations.push({ from: config.file, target: file, kind: "module-path", line: item.line, reason: "cross-owner-module-path" });
    }
  }
  for (const facade of policy.facades ?? []) {
    const scan = scans.get(facade.file);
    if (!scan) { stale.push({ key: facade.file, reason: "facade-not-scanned" }); continue; }
    const key = item => JSON.stringify([item.name, item.target, item.visibility]);
    const expected = new Set(facade.exports.map(key)), actual = new Set(scan.exports.map(key));
    for (const value of actual) if (!expected.has(value)) violations.push({ from: facade.file, target: value, kind: "export-contract", reason: "public-export-widened-or-origin-changed" });
    for (const value of expected) if (!actual.has(value)) stale.push({ key: `${facade.file}:${value}`, reason: "public-export-removed-or-origin-changed" });
    if (facade.exactModules) {
      for (const item of scan.modules) if (!facade.modules.some(expected => expected.name === item.name))
        violations.push({ from: facade.file, target: item.name, kind: "module-contract", reason: "unregistered-module-declaration" });
    }
    for (const expectedModule of facade.modules ?? []) {
      const actualModule = scan.modules.find(item => item.name === expectedModule.name);
      if (!actualModule || actualModule.visibility !== expectedModule.visibility || actualModule.path !== expectedModule.path) violations.push({ from: facade.file, target: expectedModule.name, kind: "module-contract", reason: "module-origin-or-visibility-changed" });
    }
  }
  const counts = new Map();
  for (const violation of violations) counts.set(nativeBoundaryKey(violation), (counts.get(nativeBoundaryKey(violation)) ?? 0) + 1);
  const keys = new Set();
  for (const exception of exceptions) {
    const key = nativeBoundaryKey(exception);
    if (!exception.reason || keys.has(key) || !Number.isInteger(exception.count) || exception.count < 1) { stale.push({ key, reason: "invalid-or-duplicate-exception" }); continue; }
    keys.add(key);
    if (counts.get(key) !== exception.count) stale.push({ key, reason: "exception-count-changed", expected: exception.count, actual: counts.get(key) ?? 0 });
  }
  const remaining = violations.filter(item => !exceptions.some(exception => nativeBoundaryKey(exception) === nativeBoundaryKey(item) && counts.get(nativeBoundaryKey(item)) === exception.count));
  const pending = policy.pending ?? [];
  return { ok: !remaining.length && !stale.length && !unsupported.length && !pending.length, checkedConsumers: scans.size, checkedReferences: edges.length, violations: remaining, stale, unsupported, pending, coverage: policy.coverage, testScopes: [...scans.values()].reduce((sum, item) => sum + item.testScopes, 0) };
}
