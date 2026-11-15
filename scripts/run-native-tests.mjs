import {spawnSync} from "node:child_process";
import {cp, lstat, mkdir, mkdtemp, realpath} from "node:fs/promises";
import {dirname, join, relative, resolve, sep} from "node:path";
import {fileURLToPath, pathToFileURL} from "node:url";
async function missingNativeTestPackage(id) {
  throw new Error("native_test_package_missing: provide the independently built " + id + " test package directory");
}

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const writingVariable = "RESEARCH_ASSISTANT_TEST_WRITING_V2_PACKAGES";
const literatureVariable = "RESEARCH_ASSISTANT_TEST_LITERATURE_PACKAGE";

async function fixtureDirectory(directory, temporary) {
  if (!(await lstat(directory)).isDirectory()) throw new Error("native_test_package_directory_invalid");
  const canonical = await realpath(directory);
  const descendant = relative(temporary, canonical);
  if (!descendant || descendant === ".." || descendant.startsWith(`..${sep}`)) {
    throw new Error("native_test_packages_outside_repository_tmp");
  }
  return canonical;
}

export async function prepareNativeTestEnvironment({
  root = repositoryRoot,
  environment = process.env,
  build = missingNativeTestPackage,
} = {}) {
  const temporary = join(root, ".tmp");
  await mkdir(temporary, {recursive: true});
  const canonicalTemporary = await realpath(temporary);
  const result = {...environment};
  if (environment[writingVariable] !== undefined) {
    result[writingVariable] = await fixtureDirectory(environment[writingVariable], canonicalTemporary);
  } else {
    const receipt = await build("writing_projects", ["--offline", "--project-files"]);
    if (receipt.status !== "passed") throw new Error("native_test_writing_build_failed");
    const source = await fixtureDirectory(receipt.packageRoot, canonicalTemporary);
    const fixtures = join(temporary, "runtime-plugins", "native-test-fixtures");
    await mkdir(fixtures, {recursive: true});
    const collection = await mkdtemp(join(fixtures, "writing-"));
    // Existing package acceptance fixtures consume this exact collection layout.
    await cp(source, join(collection, "package"), {recursive: true, errorOnExist: true, force: false});
    result[writingVariable] = collection;
  }
  if (environment[literatureVariable] !== undefined) {
    result[literatureVariable] = await fixtureDirectory(environment[literatureVariable], canonicalTemporary);
  } else {
    const receipt = await build("literature_library", ["--offline"]);
    if (receipt.status !== "passed") throw new Error("native_test_literature_build_failed");
    result[literatureVariable] = await fixtureDirectory(receipt.packageRoot, canonicalTemporary);
  }
  return result;
}

export async function runNativeTests(arguments_, {
  root = repositoryRoot,
  environment = process.env,
  prepare = prepareNativeTestEnvironment,
  spawn = spawnSync,
} = {}) {
  const args = arguments_[0] === "--" ? arguments_.slice(1) : arguments_;
  const fixtureEnvironment = await prepare({root, environment});
  const result = spawn("cargo", ["test", ...args], {cwd: root, env: fixtureEnvironment, stdio: "inherit"});
  if (result.error) throw result.error;
  return result.status ?? 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { process.exitCode = await runNativeTests(process.argv.slice(2)); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
