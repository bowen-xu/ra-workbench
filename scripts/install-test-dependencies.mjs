import fs from 'node:fs/promises';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const expected = JSON.parse(await fs.readFile(path.join(root, 'test-dependencies.json'), 'utf8'));
const directories = process.argv.slice(2).filter(value => value !== '--');
if (!directories.length) throw new Error('Usage: pnpm test-dependencies:install -- <native test-support artifact directories...>');
const output = path.join(root, '.dependencies/test-plugins');
await fs.mkdir(output, {recursive: true});
for (const directory of directories) {
  const metadata = JSON.parse(await fs.readFile(path.join(directory, 'native-test-support.json'), 'utf8'));
  if (metadata.formatVersion !== 1 || expected[metadata.id] !== metadata.version
    || metadata.file !== `${metadata.id}-native-test-support-${metadata.version}.tar.gz`) throw new Error('TEST_DEPENDENCY_VERSION_MISMATCH');
  const file = path.resolve(directory, metadata.file);
  if (createHash('sha256').update(await fs.readFile(file)).digest('hex') !== metadata.sha256) throw new Error('TEST_DEPENDENCY_CHANGED');
  const result = spawnSync('tar', ['-xzf', file, '-C', output], {stdio: 'inherit'});
  if (result.status !== 0) throw new Error('TEST_DEPENDENCY_EXTRACT_FAILED');
  await fs.writeFile(path.join(output, metadata.id, 'artifact.json'), JSON.stringify(metadata, null, 2) + '\n');
}
