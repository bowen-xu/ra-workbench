import { readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve } from "node:path";
const root = resolve(import.meta.dirname, "..");
const protocol = JSON.parse(await readFile(resolve(root, "apps/desktop/src-tauri/src/core/notes/text_view/protocol.json"), "utf8"));
const text = `<!-- Generated from notes/text_view/protocol.json; edit that source. -->\n${protocol.instructions}\n\nXML attribute whitelist:\n${Object.entries(protocol.tags).map(([tag, { attributes }]) => `- ${tag}: ${attributes.join(", ") || "none"}`).join("\n")}\n\nExamples:\n${protocol.examples.map(example => `\n${example.title}\n\n\`\`\`text\n${example.patch ?? example.view}\n\`\`\`\n`).join("")}`;
const paths = ["apps/desktop/src-tauri/src/core/notes/text_view/syntax.md", "apps/desktop/agent-resources/skills/edit-notes/references/text-view.md"];
for (const path of paths) {
  const absolute = resolve(root, path);
  if (process.argv.includes("--check")) {
    if (await readFile(absolute, "utf8") !== text) throw new Error(`Note protocol output is stale: ${path}`);
  } else {
    await mkdir(resolve(absolute, ".."), { recursive: true }); await writeFile(absolute, text);
  }
}
