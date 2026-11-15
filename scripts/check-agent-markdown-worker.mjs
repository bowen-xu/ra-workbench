import assert from "node:assert/strict";
import {readFile, readdir, stat} from "node:fs/promises";
import {join, resolve} from "node:path";
import {Worker} from "node:worker_threads";

// Run after a Vite production build into an ignored temporary directory:
// node scripts/check-agent-markdown-worker.mjs .tmp/markdown-worker-build/assets
// Use the actual browser worker artifact, without document/DOMParser shims.
const directory = resolve(process.argv[2] ?? ".tmp/markdown-worker-build/assets");
const artifacts = await Promise.all((await readdir(directory)).filter(name => /^markdown\.worker-.*\.js$/u.test(name))
  .map(async name => ({path: join(directory, name), time: (await stat(join(directory, name))).mtimeMs})));
artifacts.sort((a, b) => b.time - a.time);
assert.ok(artifacts[0], "Build the production Markdown worker first");
const source = await readFile(artifacts[0].path, "utf8");
const worker = new Worker(`import {parentPort} from "node:worker_threads";
globalThis.self=globalThis;
self.postMessage=value=>parentPort.postMessage(value);
${source}
parentPort.on("message", data=>self.onmessage({data}));`, {eval: true, execArgv: ["--input-type=module"]});
let revision = 0;
function render(value, reveal = false) {
  const current = ++revision;
  return new Promise((resolveReply, reject) => {
    const timer = setTimeout(() => {cleanup(); reject(new Error("MARKDOWN_WORKER_TEST_TIMEOUT"));}, 20_000);
    const onError = error => {cleanup(); reject(error);};
    const onReply = reply => {
      if (reply.revision !== current) return;
      cleanup(); resolveReply(reply);
    };
    function cleanup() {clearTimeout(timer); worker.off("message", onReply); worker.off("error", onError);}
    worker.on("message", onReply); worker.on("error", onError);
    worker.postMessage({kind: "render", owner: 1, revision: current, input: {value, reveal, streaming: true}});
  });
}
try {
  const prefix = String.raw`正文 &amp; &notin; \(x_i\)。`;
  const first = await render(`${prefix}\n\n尾部`);
  assert.equal(first.kind, "rendered");
  assert.ok(JSON.stringify(first).includes("katex"));
  assert.ok(JSON.stringify(first).includes("∉"));
  const next = await render(`${prefix}\n\n尾部新增`);
  assert.equal(next.blocks[0].revision, first.blocks[0].revision);
  assert.equal(next.blocks[0].tree, undefined);
  await render("[link][target]");
  const resolved = await render("[link][target]\n\n[target]: https://example.com");
  assert.ok(JSON.stringify(resolved).includes('"href":"https://example.com"'));
  const rich = await render("|a|b|\n|-|-|\n|1|2|\n\n```js\nconst x=1;\n```\n\n" + String.raw`\[\frac{a}{b}\]`, true);
  assert.equal(rich.kind, "rendered");
  assert.ok(JSON.stringify(rich).includes("stream-reveal-glyph"));
  assert.ok(JSON.stringify(rich).includes("katex-display"));
  const equality = await render(String.raw`\[
\operatorname{score}(r)
=
\operatorname{coverage}(r)
-\lambda\,\operatorname{complexity}(r)
\]`, true);
  assert.equal(equality.kind, "rendered");
  assert.ok(JSON.stringify(equality).includes("katex-display"));
  assert.ok(!JSON.stringify(equality).includes('"tagName":"h1"'));
  const malicious = await render('<script>alert(1)</script>\n\n[link](javascript:alert)');
  assert.ok(!JSON.stringify(malicious).includes('"tagName":"script"'));
  assert.ok(!JSON.stringify(malicious).includes('"href":"javascript:'));
  worker.postMessage({kind: "release", owner: 1});
  const reopened = await render(prefix);
  assert.ok(reopened.blocks[0].tree, "Released caches cannot be referenced by a new client");
  console.log("PASS: production Worker without DOM; entities, KaTeX, block math with standalone equality, GFM, code, reveal, prefix reuse, reference invalidation, URL/HTML safety and cache release");
} finally {await worker.terminate();}
