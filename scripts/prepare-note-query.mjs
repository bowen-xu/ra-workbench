import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath, pathToFileURL} from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/** Consume the SDK's versioned pure query kernel. Core owns its emitted URL. */
export function prepareNoteQuery() {
  const source = path.join(root, '.dependencies/sdk/note-query/web');
  const receipt = JSON.parse(fs.readFileSync(path.join(source, 'receipt.json'), 'utf8'));
  const output = path.join(root, 'apps/web/src/.generated/note-query');
  fs.mkdirSync(output, {recursive: true});
  for (const [file, expected] of Object.entries(receipt.artifactHashes)) {
    if (!['research_note_query_wasm.js', 'research_note_query_wasm_bg.wasm', 'research_note_query_wasm.d.ts'].includes(file)) throw new Error('note_query_artifact_invalid');
    const bytes = fs.readFileSync(path.join(source, file));
    if (createHash('sha256').update(bytes).digest('hex') !== expected) throw new Error('note_query_artifact_changed: ' + file);
    fs.writeFileSync(path.join(output, file), bytes);
  }
  fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n');
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) prepareNoteQuery();
