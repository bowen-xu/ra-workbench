// SPDX-License-Identifier: AGPL-3.0-or-later
import fs from "node:fs/promises";
import path from "node:path";
import ts from "typescript";

const app = path.resolve(import.meta.dirname, "..");
const root = path.resolve(app, "../..");
const output = path.join(root, ".tmp/chrome-connector");
const preview = path.join(app, "preview");
const pages = new Set(["research-assistant/options.html", "progressWindow/progressWindow.html", "itemSelector/itemSelector.html"]);
const types = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".gif": "image/gif", ".md": "text/plain; charset=utf-8" };
const csp = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'none'; frame-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'";

async function sourceFile(base, relative) {
  const file = path.resolve(base, relative);
  if (!file.startsWith(`${base}${path.sep}`)) throw new Error("Invalid preview asset");
  const real = await fs.realpath(file);
  if (!real.startsWith(`${base}${path.sep}`)) throw new Error("Invalid preview asset");
  return fs.readFile(real);
}

/** Serves unchanged production UI assets with preview-only platform ports injected. */
export async function createPreviewHandler() {
  await fs.access(path.join(output, "manifest.json"));
  const strings = await fs.readFile(path.join(output, "_locales/en/messages.json"), "utf8");
  const tokens = (await fs.readFile(path.join(root, "apps/web/src/core/styles.css"), "utf8"))
    .match(/^\s*--ui-font-(?:caption|meta|label|body|heading):[^;]+;/gm)?.join("\n");
  if (!tokens) throw new Error("Shared typography tokens missing");
  return async (request, response) => {
    response.setHeader("Cache-Control", "no-store");
    response.setHeader("Content-Security-Policy", csp);
    response.setHeader("Cross-Origin-Resource-Policy", "same-origin");
    response.setHeader("X-Content-Type-Options", "nosniff");
    if (!/^127\.0\.0\.1:\d+$/.test(request.headers.host ?? "")) {
      response.writeHead(400).end(); return;
    }
    if (!["GET", "HEAD"].includes(request.method)) {
      response.writeHead(405).end(); return;
    }
    try {
      const url = new URL(request.url, `http://${request.headers.host}`);
      const pathname = decodeURIComponent(url.pathname);
      let file = pathname, body;
      if (pathname === "/") { file = "index.html"; body = await sourceFile(preview, file); }
      else if (pathname === "/preview/tokens.css") { body = `:root {\n${tokens}\n}`; }
      else if (["/preview/fixtures.js", "/preview/platform.js", "/preview/harness.js"].includes(pathname)) {
        const source = await sourceFile(preview, `${path.basename(pathname, ".js")}.ts`);
        body = ts.transpileModule(source.toString(), { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText;
        if (pathname === "/preview/platform.js") body = `window.connectorPreviewStrings = ${strings};\n${body}`;
      } else if (pathname === "/preview/harness.css") { body = await sourceFile(preview, "harness.css"); }
      else if (pathname.startsWith("/extension/")) {
        file = pathname.slice("/extension/".length);
        body = await sourceFile(output, file);
        if (pages.has(file)) {
          const original = body.toString();
          if (!original.includes("<head>")) throw new Error("Production UI head changed");
          // Only adapters are added. All markup, component scripts and styles are production files.
          body = original.replace("<head>", '<head><script src="/preview/fixtures.js"></script><script src="/preview/platform.js"></script>');
        } else if (path.extname(file) === ".html") { response.writeHead(404).end(); return; }
      } else { response.writeHead(404).end(); return; }
      response.setHeader("Content-Type", types[path.extname(file)] ?? "application/octet-stream");
      response.writeHead(200).end(request.method === "HEAD" ? undefined : body);
    } catch {
      response.writeHead(404).end("Preview asset unavailable");
    }
  };
}
