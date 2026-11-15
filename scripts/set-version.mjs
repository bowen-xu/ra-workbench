import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const version = process.argv[2];
const semverPattern = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

if (version === undefined || !semverPattern.test(version)) {
  throw new Error("Usage: pnpm version:set <semver>, for example 0.1.1");
}

const root = process.cwd();
const jsonFiles = [
  "package.json",
  "apps/web/package.json",
  "apps/desktop/package.json",
  "apps/desktop/src-tauri/tauri.conf.json",
];

const cargoPath = resolve(root, "apps/desktop/src-tauri/Cargo.toml");
const cargoToml = await readFile(cargoPath, "utf8");
const packageSection = cargoToml.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[0];
const versionPattern = /^([ \t]*version\s*=\s*")[^"]+(".*)$/m;
// A same-version retry is valid. Detect a missing version field independently
// of byte changes, and validate it before modifying the JSON metadata.
if (!packageSection || !versionPattern.test(packageSection)) {
  throw new Error("Could not update the package version in Cargo.toml");
}
const updatedPackage = packageSection.replace(versionPattern, (_match, prefix, suffix) => `${prefix}${version}${suffix}`);
const updatedCargoToml = cargoToml.replace(packageSection, updatedPackage);

for (const relativePath of jsonFiles) {
  const path = resolve(root, relativePath);
  const document = JSON.parse(await readFile(path, "utf8"));
  document.version = version;
  await writeFile(path, `${JSON.stringify(document, null, 2)}\n`);
}

await writeFile(cargoPath, updatedCargoToml);
console.log(`Research Assistant version set to ${version}`);
