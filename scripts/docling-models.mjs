import { createHash } from "node:crypto";
import { createReadStream, createWriteStream } from "node:fs";
import {
  mkdir,
  readFile,
  writeFile,
  rename,
  rm,
  stat,
  realpath,
  mkdtemp,
  readdir,
} from "node:fs/promises";
import { resolve, dirname, sep } from "node:path";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";
import { pathToFileURL } from "node:url";

export const ENGINE_VERSION = "2.45.7";
export const ENGINE_COMMIT = "a90192cd3096b5db26db928ded4d3f024d7e4ed4";
export const REQUIRED_MODELS = [
  "layout_heron.onnx",
  "ocr_det.onnx",
  "ocr_rec_v6.onnx",
  "ocr_rec_v6_dict.txt",
  "ocr_textline_ori.onnx",
  "tableformer/encoder.onnx",
  "tableformer/decoder.onnx",
  "tableformer/bbox.onnx",
];
const sidecars = [
  "tableformer/encoder.onnx.data",
  "tableformer/decoder.onnx.data",
  "tableformer/bbox.onnx.data",
];
export async function hashModel(path) {
  const digest = createHash("sha256");
  let sizeBytes = 0;
  for await (const bytes of createReadStream(path)) {
    digest.update(bytes);
    sizeBytes += bytes.length;
  }
  return { sha256: digest.digest("hex"), sizeBytes };
}
export function modelBundleName(files) {
  return createHash("sha256")
    .update(
      JSON.stringify([
        ENGINE_VERSION,
        ENGINE_COMMIT,
        "fp32",
        [...files].sort((a, b) =>
          a.path < b.path ? -1 : a.path > b.path ? 1 : 0,
        ),
      ]),
    )
    .digest("hex");
}
/** Publish only explicit model data and license files; never copy executable or arbitrary source files. */
export async function publishModels({ root = process.cwd(), source, version }) {
  if (!source || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/.test(version ?? ""))
    throw new Error("Use --source <FP32 directory> --version <model version>");
  const input = await realpath(resolve(source));
  const entries = [...REQUIRED_MODELS];
  for (const file of sidecars) {
    try {
      await stat(resolve(input, file));
      entries.push(file);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
  const licenses = (await readdir(resolve(input, "licenses"))).filter((name) =>
    /^[A-Za-z0-9._-]+$/.test(name),
  );
  if (!licenses.length)
    throw new Error("Model license files are required under source/licenses/");
  entries.push(...licenses.map((name) => `licenses/${name}`));
  if (entries.length > 32) throw new Error("Too many model resource files");
  const files = [];
  for (const path of entries) {
    const canonical = await realpath(resolve(input, path));
    if (!canonical.startsWith(`${input}${sep}`))
      throw new Error("Model resource escapes source directory");
    const metadata = await hashModel(canonical);
    if (!metadata.sizeBytes || metadata.sizeBytes > 2 * 1024 ** 3)
      throw new Error(`Invalid model size: ${path}`);
    files.push({ path, ...metadata });
  }
  if (files.reduce((sum, f) => sum + f.sizeBytes, 0) > 4 * 1024 ** 3)
    throw new Error("Model resources exceed 4 GiB");
  const manifest = {
    schemaVersion: 1,
    engineVersion: ENGINE_VERSION,
    engineCommit: ENGINE_COMMIT,
    precision: "fp32",
    version,
    files,
  };
  const name = modelBundleName(files);
  const destination = resolve(root, ".release/models/docling");
  await mkdir(destination, { recursive: true });
  const staging = await mkdtemp(resolve(destination, ".staging-"));
  try {
    for (const file of files) {
      const target = resolve(staging, file.path);
      await mkdir(dirname(target), { recursive: true });
      await pipeline(
        createReadStream(resolve(input, file.path)),
        createWriteStream(target, { flags: "wx" }),
      );
      const actual = await hashModel(target);
      if (actual.sha256 !== file.sha256 || actual.sizeBytes !== file.sizeBytes)
        throw new Error(`Resource changed during publication: ${file.path}`);
    }
    await writeFile(
      resolve(staging, "manifest.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );
    try {
      await rename(staging, resolve(destination, name));
    } catch (error) {
      if (!["EEXIST", "ENOTEMPTY"].includes(error.code)) throw error;
      for (const file of files) {
        const existing = await hashModel(resolve(destination, name, file.path));
        if (
          existing.sha256 !== file.sha256 ||
          existing.sizeBytes !== file.sizeBytes
        )
          throw new Error("Existing model bundle is corrupt");
      }
    }
    const pending = resolve(destination, `.latest-${process.pid}.json`);
    await writeFile(pending, `${JSON.stringify(manifest, null, 2)}\n`);
    await rename(pending, resolve(destination, "latest.json"));
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
  return { manifest, name, directory: resolve(destination, name) };
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  try {
    const args = process.argv.slice(2).filter((a) => a !== "--");
    const option = (name) => args[args.indexOf(name) + 1];
    const result = await publishModels({
      source: args.includes("--source") ? option("--source") : undefined,
      version: args.includes("--version") ? option("--version") : undefined,
    });
    console.log(
      `FP32 models ${result.manifest.version}: ${result.directory}\nServe with pnpm release:serve`,
    );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
