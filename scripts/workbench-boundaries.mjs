import {logicalProjectPath, projectSources} from "./project-layout.mjs";
import { existsSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, extname, relative, resolve, sep } from "node:path";

// Reuse the installed compiler and its resolver, including tsconfig path aliases.
const require = createRequire(new URL("../apps/web/package.json", import.meta.url));
const ts = require("typescript");
const prefix = "apps/web/src/";
const toPosix = value => value.split(sep).join("/");
const platformImport = name => name.startsWith("@tauri-apps/");
const rootPluginSource = file => /^plugins\/([^/]+)\/src\//u.exec(file);
const workbenchSource = file => file.startsWith(prefix) || file.startsWith("tests/") || file.startsWith("preview-host/") || Boolean(rootPluginSource(file));

function owner(file) {
  if (file.startsWith("preview-host/")) return "preview-host";
  if (file.startsWith("tests/") || file.startsWith(prefix) && !file.startsWith(prefix + "plugins/") && /\.(?:test|spec|browser|testSupport)\.tsx?$/u.test(file)) return "core-test";
  const rootPlugin = rootPluginSource(file);
  if (rootPlugin) return `plugins/${rootPlugin[1]}`;
  if (!file.startsWith(prefix)) return "consumer";
  const path = file.slice(prefix.length);
  if (/^(?:core|assets|shims|\.generated)\//.test(path)) return "core";
  const plugin = path.match(/^plugins\/([^/]+)\//);
  if (plugin) return `plugins/${plugin[1]}`;
  return "app";
}

/** Source lists are repository-relative. A pure contract is admitted only by
 * its exact policy path, never by a public/contracts directory prefix. */
export function findExcludedWorkbenchPluginSources(sources, selectedIds, contracts = []) {
  const selected = new Set(selectedIds), pureContracts = new Set(contracts);
  return sources.filter(source => {
    const path = source.replaceAll("\\", "/"), sourceOwner = owner(path);
    return sourceOwner.startsWith("plugins/") && !selected.has(sourceOwner.slice("plugins/".length)) && !pureContracts.has(path);
  });
}

function walk(directory) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    if (["node_modules", ".generated"].includes(entry.name)) return [];
    const file = resolve(directory, entry.name);
    if (entry.isDirectory()) return walk(file);
    return /\.(?:[cm]?[jt]sx?|css)$/.test(entry.name) ? [file] : [];
  });
}

function namedBindings(node) {
  if (ts.isImportDeclaration(node)) {
    const clause = node.importClause;
    if (!clause) return "side-effect";
    const names = clause.name ? ["default"] : [];
    if (clause.namedBindings) {
      if (ts.isNamespaceImport(clause.namedBindings)) names.push("*");
      else names.push(...clause.namedBindings.elements.map(element =>
        `${clause.isTypeOnly || element.isTypeOnly ? "type:" : ""}${(element.propertyName ?? element.name).text}`));
    }
    return names.sort().join(",");
  }
  if (ts.isExportDeclaration(node)) {
    if (!node.exportClause) return "*";
    if (ts.isNamespaceExport(node.exportClause)) return "*";
    return node.exportClause.elements.map(element =>
      `${node.isTypeOnly || element.isTypeOnly ? "type:" : ""}${(element.propertyName ?? element.name).text}`).sort().join(",");
  }
  return "*";
}

function isTypeOnly(node) {
  if (ts.isImportTypeNode(node)) return true;
  if (ts.isImportDeclaration(node)) {
    const clause = node.importClause;
    return Boolean(clause?.isTypeOnly || (clause && !clause.name && clause.namedBindings &&
      ts.isNamedImports(clause.namedBindings) && clause.namedBindings.elements.length &&
      clause.namedBindings.elements.every(element => element.isTypeOnly)));
  }
  return ts.isExportDeclaration(node) && Boolean(node.isTypeOnly ||
    (node.exportClause && ts.isNamedExports(node.exportClause) && node.exportClause.elements.length &&
      node.exportClause.elements.every(element => element.isTypeOnly)));
}

function enclosingFunctionName(node) {
  for (let parent = node.parent; parent; parent = parent.parent) {
    if (!ts.isFunctionLike(parent)) continue;
    if (parent.name && ts.isIdentifier(parent.name)) return parent.name.text;
    if ((ts.isArrowFunction(parent) || ts.isFunctionExpression(parent)) && ts.isVariableDeclaration(parent.parent)
      && ts.isIdentifier(parent.parent.name)) return parent.parent.name.text;
    // A nested anonymous callback is its own site, not its named outer owner.
    return "<anonymous>";
  }
}

/** Collect literal dependencies, resource imports, mocks and computed import sites.
 * This is an import-boundary check, not a proof of process or permission isolation.
 */
export function collectWorkbenchImports(root, directories) {
  root = realpathSync(root);
  if (directories === undefined) {
    const plugins = resolve(root, "plugins");
    const pluginSources = existsSync(plugins) ? readdirSync(plugins, {withFileTypes: true})
      .filter(entry => !projectSources.some(([physical]) => physical === `plugins/${entry.name}`)).filter(entry => entry.isDirectory() || (entry.isSymbolicLink() && ["literature_library", "note_manager", "writing_projects"].includes(entry.name))).map(entry => `plugins/${entry.name}/src`) : [];
    directories = ["apps/web/src", "preview-host", ...pluginSources, "tests", "docs/ui-templates/reorder-template"];
  }
  const configFile = resolve(root, "apps/web/tsconfig.app.json");
  const config = existsSync(configFile) ? ts.readConfigFile(configFile, ts.sys.readFile) : { config: {} };
  if (config.error) throw new Error(ts.flattenDiagnosticMessageText(config.error.messageText, "\n"));
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, dirname(configFile));
  const options = { moduleResolution: ts.ModuleResolutionKind.Bundler, module: ts.ModuleKind.ESNext, ...parsed.options };
  const cache = ts.createModuleResolutionCache(root, value => value, options);
  const edges = [];
  const record = (file, specifier, kind, names, typeOnly, line, enclosingFunction) => {
    const from = logicalProjectPath(root, realpathSync(file));
    if (specifier === null) {
      edges.push({ from, target: "<computed>", kind, names, typeOnly, line, computed: true,
        ...(enclosingFunction === undefined ? {} : {enclosingFunction}) });
      return;
    }
    let target;
    const clean = specifier.split(/[?#]/)[0];
    const moduleName = (clean.startsWith("/plugins/literature_library/src/") || clean.startsWith("/plugins/writing_projects/src/")) ? resolve(root, clean.slice(1))
      : clean.startsWith("/src/") ? resolve(root, "apps/web", clean.slice(1))
      : clean.startsWith("/@fs/") ? clean.slice(4) : clean;
    // This exact generated entry belongs to App composition, even though the
    // TypeScript resolver sees only its ambient declaration. It must not be
    // treated as an external package that Core/plugins could import freely.
    if (clean === "virtual:workbench-composition") target = resolve(root, prefix + "app/workbenchComposition.ts");
    // A URL loads the actual resource, not the .d.ts file TypeScript may prefer.
    else if (kind === "resource-url" && (moduleName.startsWith(".") || moduleName.startsWith("/"))) {
      const resource = resolve(dirname(file), moduleName);
      if (existsSync(resource)) target = resource;
    }
    const resolution = target ? undefined : ts.resolveModuleName(moduleName, file, options, ts.sys, cache).resolvedModule;
    if (resolution) target = resolution.resolvedFileName;
    if (!target && (moduleName.startsWith(".") || moduleName.startsWith("/"))) {
      const resource = resolve(dirname(file), moduleName);
      if (existsSync(resource)) target = resource;
    }
    if (target) target = logicalProjectPath(root, realpathSync(target));
    // Published package declarations retain the existing logical contract
    // identity after pnpm installs them under a content-addressed directory.
    const publicName = /^@research-assistant\/([^/]+)(?:\/|$)/u.exec(clean)?.[1];
    if (resolution?.isExternalLibraryImport && publicName && resolution.packageId?.name === "@research-assistant/" + publicName) {
      const installedPrefix = "/node_modules/@research-assistant/" + publicName + "/lib/";
      const resolved = toPosix(realpathSync(resolution.resolvedFileName));
      const offset = resolved.lastIndexOf(installedPrefix);
      if (offset >= 0) target = "packages/" + publicName + "/src/" + resolved.slice(offset + installedPrefix.length).replace(/\.d\.ts$/u, ".ts");
    }
    if (platformImport(specifier)) target = specifier;
    // Retain external imports too: contract files must never import runtime code.
    target ??= specifier;
    edges.push({ from, target, kind, names, typeOnly, line, specifier });
  };
  for (const file of directories.flatMap(directory => walk(resolve(root, directory))).sort()) {
    const text = readFileSync(file, "utf8");
    if (extname(file) === ".css") {
      // Ignore comments and keep offsets stable for actionable diagnostics.
      const css = text.replace(/\/\*[\s\S]*?\*\//g, value => value.replace(/[^\n]/g, " "));
      for (const match of css.matchAll(/@import\s+(?:url\(\s*)?(?:(["'])(.*?)\1|([^\s);]+))/g))
        record(file, match[2] ?? match[3], "css-import", "*", false, css.slice(0, match.index).split("\n").length);
      continue;
    }
    const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
    if (/\/contracts\/|\/contracts\.ts$/.test(toPosix(file))) {
      for (const statement of source.statements) {
        if (ts.isImportDeclaration(statement) || ts.isExportDeclaration(statement) || ts.isInterfaceDeclaration(statement) || ts.isTypeAliasDeclaration(statement)) continue;
        edges.push({from: toPosix(relative(root, file)), target: "<runtime>", kind: "declaration",
          names: ts.SyntaxKind[statement.kind], typeOnly: false,
          line: source.getLineAndCharacterOfPosition(statement.getStart(source)).line + 1});
      }
    }
    const add = (node, specifier, kind) => record(file,
      ts.isStringLiteralLike(specifier) ? specifier.text : null, kind,
      ts.isStringLiteralLike(specifier) ? namedBindings(node) : specifier.getText(source),
      isTypeOnly(node), source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1,
      ts.isStringLiteralLike(specifier) ? undefined : enclosingFunctionName(node));
    function visit(node) {
      if (ts.isImportDeclaration(node)) add(node, node.moduleSpecifier, "import");
      else if (ts.isExportDeclaration(node) && node.moduleSpecifier) add(node, node.moduleSpecifier, "export");
      else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) add(node, node.argument.literal, "import-type");
      else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference) && node.moduleReference.expression)
        add(node, node.moduleReference.expression, "require");
      else if (ts.isNewExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "URL" &&
        node.arguments?.[0] && node.arguments[1]?.getText(source) === "import.meta.url") add(node, node.arguments[0], "resource-url");
      else if (ts.isCallExpression(node) && node.arguments[0]) {
        if (node.expression.kind === ts.SyntaxKind.ImportKeyword) add(node, node.arguments[0], "dynamic-import");
        else if (ts.isIdentifier(node.expression) && node.expression.text === "require") add(node, node.arguments[0], "require");
        else if (ts.isPropertyAccessExpression(node.expression) && ts.isIdentifier(node.expression.expression) &&
          ["vi", "vitest"].includes(node.expression.expression.text) && ["mock", "doMock", "importActual"].includes(node.expression.name.text))
          add(node, node.arguments[0], "mock");
      }
      ts.forEachChild(node, visit);
    }
    visit(source);
  }
  return edges;
}

/** One exception is one concrete source/target/kind/imported-name set, with an
 * occurrence budget. Formatting and line movement do not silently widen it.
 */
export function boundaryKey(edge) {
  return JSON.stringify([edge.from, edge.target, edge.kind, edge.names]);
}

function runtimeImportPortKey(source, kind, expression, functionName) {
  return JSON.stringify([source, kind, expression, functionName]);
}

export function classifyBoundary(edge, policy) {
  const sourceOwner = owner(edge.from), targetOwner = owner(edge.target);
  const sourceContract = policy.contracts.includes(edge.from);
  const sourcePublic = policy.publicEntries.includes(edge.from);
  const targetContract = policy.contracts.includes(edge.target);
  const targetPublic = policy.publicEntries.includes(edge.target);
  if (sourceContract) {
    if (edge.kind === "declaration") return "contract-runtime-code";
    if (edge.computed || !edge.typeOnly) return "contract-runtime-import";
    const publicPackageType = edge.target.startsWith("packages/") && !edge.specifier?.startsWith(".") && edge.target !== edge.specifier &&
      (edge.specifier?.startsWith("@research-assistant/") && !edge.specifier.includes("/src/"));
    if (!targetContract && !publicPackageType) return "contract-private-import";
  }
  if (sourcePublic && edge.kind === "export" && edge.names === "*") return "public-star-export";
  if (edge.computed) return sourceOwner === "core-test" && /^tests\/ui\//u.test(edge.from) ? null : "computed-import";
  if (platformImport(edge.target)) return sourceOwner.startsWith("plugins/") ? "plugin-platform-import" : null;
  if (!workbenchSource(edge.target)) return null;
  // Test composition is never a production API, including inside App itself.
  if (policy.testAssemblyEntries?.includes(edge.target))
    return sourceOwner === "preview-host" || /\.(?:test|spec)\.tsx?$/.test(edge.from) && edge.kind === "import" ? null : "application-reverse-import";
  if (policy.browserTestEntries?.includes(edge.target)) {
    const browserDriver = /^tests\/ui\/.+\.pw\.ts$/.test(edge.from)
      || policy.browserTestHelpers?.includes(edge.from);
    return browserDriver && (edge.kind === "dynamic-import" || edge.typeOnly) ? null : "application-reverse-import";
  }
  if (targetOwner === "core-test" && !["core-test", "preview-host"].includes(sourceOwner)) return "production-test-import";
  if (targetOwner === "preview-host" && !["core-test", "preview-host"].includes(sourceOwner)) return "production-preview-import";
  if (["core-test", "preview-host"].includes(sourceOwner)) {
    if (targetOwner.startsWith("plugins/")) return "core-plugin-implementation";
    if (["core", "app", "core-test", "preview-host"].includes(targetOwner)) return null;
  }
  if (sourceOwner === targetOwner) return null;
  if (targetOwner === "app") {
    return "application-reverse-import";
  }
  if (sourceOwner === "core" && targetOwner.startsWith("plugins/"))
    return edge.typeOnly && targetContract || edge.kind === "mock" && /\.(?:test|spec)\./.test(edge.from) && targetPublic ? null : "core-plugin-implementation";
  // The composition root owns concrete Core adapter selection. It is not a
  // plugin API: Core and plugins cannot depend back on application code.
  if (sourceOwner === "app") return targetOwner === "core" || targetPublic || (edge.typeOnly && targetContract) ? null : "assembly-private-import";
  if (sourcePublic) return targetPublic || (edge.typeOnly && targetContract) ? null : "public-cross-owner-implementation";
  if (sourceOwner.startsWith("plugins/"))
    return targetPublic || (edge.typeOnly && targetContract) ? null : "plugin-private-import";
  return targetPublic || (edge.typeOnly && targetContract) ? null : "consumer-private-import";
}

export function checkWorkbenchBoundaries(edges, policy, baseline) {
  if (policy.schemaVersion !== 1 || baseline.schemaVersion !== 1) throw new Error("Unsupported boundary policy schema");
  for (const name of ["publicEntries", "contracts"])
    if (!Array.isArray(policy[name]) || new Set(policy[name]).size !== policy[name].length)
      throw new Error(`Invalid boundary policy ${name}`);
  if ([...policy.publicEntries, ...policy.contracts].some(path => !workbenchSource(path) || path.includes("*") || path.includes("..")))
    throw new Error("Boundary surfaces must name exact Web or root plugin source files");
  if (policy.publicEntries.some(path => policy.contracts.includes(path))) throw new Error("A contract cannot also be a runtime entry");
  const browserTestEntries = policy.browserTestEntries ?? [];
  if (!Array.isArray(browserTestEntries) || new Set(browserTestEntries).size !== browserTestEntries.length ||
    browserTestEntries.some(path => typeof path !== "string" || !/^apps\/web\/src\/app\/.+\.browser\.tsx$/.test(path) || path.includes("*") || path.includes("..")))
    throw new Error("Browser test entries must name exact application .browser.tsx files");
  if (browserTestEntries.some(path => policy.publicEntries.includes(path) || policy.contracts.includes(path)))
    throw new Error("A browser test entry is not a plugin API or contract");
  const browserTestHelpers = policy.browserTestHelpers ?? [];
  if (!Array.isArray(browserTestHelpers) || new Set(browserTestHelpers).size !== browserTestHelpers.length ||
    browserTestHelpers.some(path => typeof path !== "string" || !/^tests\/ui\/[a-zA-Z0-9_-]+\.ts$/.test(path)))
    throw new Error("Browser test helpers must name exact tests/ui helper files");
  const testAssemblyEntries = policy.testAssemblyEntries ?? [];
  if (!Array.isArray(testAssemblyEntries) || new Set(testAssemblyEntries).size !== testAssemblyEntries.length ||
    testAssemblyEntries.some(path => typeof path !== "string" || !/^apps\/web\/src\/app\/(?:[a-zA-Z0-9_-]+\/)*[a-zA-Z0-9_-]+\.testSupport\.tsx$/.test(path)))
    throw new Error("Test assembly entries must name exact application .testSupport.tsx files");
  if (testAssemblyEntries.some(path => policy.publicEntries.includes(path) || policy.contracts.includes(path) || browserTestEntries.includes(path)))
    throw new Error("A test assembly entry is not a plugin API, contract or browser driver entry");
  const runtimeImportPorts = policy.runtimeImportPorts ?? [];
  if (!Array.isArray(runtimeImportPorts)) throw new Error("Runtime import ports must be an array of exact Core sites");
  const runtimePorts = new Map();
  for (const port of runtimeImportPorts) {
    if (!port || typeof port !== "object" || Array.isArray(port) || typeof port.source !== "string"
      || !/^apps\/web\/src\/core\/(?:[a-zA-Z0-9_-]+\/)*[a-zA-Z0-9_-]+\.tsx?$/.test(port.source)
      || /\.(?:test|spec)\./.test(port.source) || port.kind !== "dynamic-import"
      || typeof port.expression !== "string" || !/^[a-zA-Z_$][\w$]*$/.test(port.expression)
      || typeof port.function !== "string" || !/^[a-zA-Z_$][\w$]*$/.test(port.function)
      || port.count !== 1 || typeof port.description !== "string" || !port.description.trim())
      throw new Error("Runtime import ports must name an exact Core dynamic-import expression/function with budget 1");
    if (policy.publicEntries.includes(port.source) || policy.contracts.includes(port.source)
      || port.source.startsWith(prefix + "core/public/") || /\/contracts(?:\/|\.tsx?$)/.test(port.source))
      throw new Error("A runtime import port cannot be a public entry or contract");
    const key = runtimeImportPortKey(port.source, port.kind, port.expression, port.function);
    if (runtimePorts.has(key)) throw new Error("Duplicate runtime import port " + key);
    runtimePorts.set(key, port);
  }
  const allowance = new Map();
  for (const [from, target, kind, names, count, workItem] of baseline.exceptions) {
    if (!Number.isInteger(count) || count < 1 || !baseline.workItems[workItem]) throw new Error("Invalid boundary exception");
    const key = boundaryKey({ from, target, kind, names });
    if (allowance.has(key)) throw new Error(`Duplicate boundary exception ${key}`);
    allowance.set(key, { count, workItem });
  }
  const violations = [], active = new Map(), runtimeCounts = new Map();
  for (const edge of edges) {
    const reason = classifyBoundary(edge, policy);
    if (!reason) continue;
    // This permanent reviewed Core port is separate from migration exceptions.
    // Match the expression and its nearest function as well as file/kind; an
    // excess occurrence is never rescued by a larger legacy import budget.
    if (reason === "computed-import" && runtimeImportPorts.some(port => port.source === edge.from)) {
      const portKey = runtimeImportPortKey(edge.from, edge.kind, edge.names, edge.enclosingFunction);
      const port = runtimePorts.get(portKey), count = (runtimeCounts.get(portKey) ?? 0) + 1;
      runtimeCounts.set(portKey, count);
      if (!port || count > port.count) violations.push({...edge, reason});
      continue;
    }
    const key = boundaryKey(edge), count = (active.get(key) ?? 0) + 1;
    active.set(key, count);
    // New public and contract files must be clean from their first introduction.
    const freshSurface = policy.publicEntries.includes(edge.from) || policy.contracts.includes(edge.from);
    const isolatedCore = ["core-test", "preview-host"].includes(owner(edge.from)) && reason === "core-plugin-implementation";
    const coreImplementation = edge.from.startsWith(prefix + "core/") && !/\.(?:test|spec)\./.test(edge.from)
      && ["core-plugin-implementation", "application-reverse-import"].includes(reason);
    if (freshSurface || isolatedCore || coreImplementation || count > (allowance.get(key)?.count ?? 0)) violations.push({ ...edge, reason });
  }
  const stale = [...allowance].filter(([key, value]) => (active.get(key) ?? 0) < value.count)
    .map(([key, value]) => ({ key, allowed: value.count, current: active.get(key) ?? 0, workItem: value.workItem }));
  return { ok: !violations.length && !stale.length, violations, stale,
    checkedImports: edges.length, legacyReferences: [...active.values()].reduce((sum, value) => sum + value, 0) };
}
