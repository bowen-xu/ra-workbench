// SPDX-License-Identifier: AGPL-3.0-or-later
async function runCspRegression() {
  const rows = document.querySelector("#results");
  let failures = 0;
  const check = (condition, label) => {
    const row = document.createElement("li");
    row.textContent = `${condition ? "PASS" : "FAIL"} — ${label}`;
    rows.append(row);
    if (!condition) failures++;
  };
  const policy = document.querySelector('meta[http-equiv="Content-Security-Policy"]').content;
  const unsupported = policy.split(";").map(directive => directive.trim().split(/\s+/)[0])
    .filter(name => ["frame-ancestors", "sandbox", "report-uri"].includes(name));
  check(unsupported.length === 0, `unsupported meta directives: ${unsupported.join(", ") || "none"}`);
  // CSP events are asynchronous. Await each deliberate violation, rather than
  // relying on arbitrary delays to decide whether the initial parse was quiet.
  async function blocked(directive, action) {
    let listener;
    let timer;
    try {
      await new Promise((resolve, reject) => {
        listener = event => { if (event.effectiveDirective === directive) resolve(); };
        document.addEventListener("securitypolicyviolation", listener);
        timer = setTimeout(() => reject(new Error(`No ${directive} enforcement event`)), 3000);
        action();
      });
      check(true, `${directive} blocked`);
    } catch (error) { check(false, error.message); }
    finally { clearTimeout(timer); document.removeEventListener("securitypolicyviolation", listener); }
  }
  const html = `<!doctype html><html><head>
    <base href="/papers/"><meta name="citation_title" content="CSP capture fixture">
    <meta name="citation_doi" content="10.1000/csp-fixture">
    <style>.article { color: green; }</style></head><body style="color: red">
    <article class="article" style="display: none"><a href="paper.pdf">PDF</a></article>
    </body></html>`;
  Zotero.OffscreenTranslate.init();
  cspMessages.get("Translate.new")(1, 1);
  check(cspMessages.get("Translate.setDocument")(html, "https://paper.invalid/article", "", 1, 1) === true, "actual setDocument handler completes");
  const documentCopy = Zotero.OffscreenTranslate.translateInstances[1][1].document;
  check(documentCopy.querySelector('meta[name="citation_title"]').content === "CSP capture fixture"
    && documentCopy.querySelector('meta[name="citation_doi"]').content === "10.1000/csp-fixture", "citation metadata preserved");
  check(documentCopy.body.style.color === "red" && documentCopy.querySelector("article").style.display === "none", "inline CSS remains readable by translators");
  check(documentCopy.querySelector("style").textContent.includes("color: green"), "style element preserved");
  check(documentCopy.querySelector("a").href === "https://paper.invalid/papers/paper.pdf", "upstream base URL resolution preserved");
  const response = await requestOverride.local.postReceive({ response: html, responseType: "document", responseHeaders: "Content-Type: text/html" });
  check(response.response.querySelector('meta[name="citation_title"]').content === "CSP capture fixture"
    && response.response.body.style.color === "red", "actual HTTP document parser preserves metadata and inline CSS");
  await blocked("connect-src", () => { void fetch("/blocked/fetch").catch(() => undefined); });
  await blocked("img-src", () => { const image = document.createElement("img"); image.src = "/blocked/image"; document.body.append(image); });
  await blocked("style-src-elem", () => { const link = document.createElement("link"); link.rel = "stylesheet"; link.href = "/blocked/style.css"; document.head.append(link); });
  await blocked("script-src-elem", () => { const script = document.createElement("script"); script.textContent = "document.body.dataset.inlineScriptRan = 'yes'"; document.body.append(script); });
  check(!document.body.dataset.inlineScriptRan, "inline script did not execute");
  const inlineStyleViolations = cspViolations.filter(event => event.directive.startsWith("style-src") && event.blocked === "inline");
  check(inlineStyleViolations.length === 0, `inline style CSP violations: ${inlineStyleViolations.length}`);
  document.querySelector("#status").textContent = failures ? `FAIL — ${failures} checks` : "PASS — all checks";
}
runCspRegression().catch(error => { document.querySelector("#status").textContent = `FAIL — ${error.message}`; });
