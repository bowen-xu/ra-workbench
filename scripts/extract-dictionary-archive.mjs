import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdir, mkdtemp, rename, rm } from "node:fs/promises";
import { dirname, basename, resolve } from "node:path";
import SevenZip from "7z-wasm";

async function fileDigest(path) {
  const hash = createHash("sha256");
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest("hex");
}

/** Build-only extractor. Mount only pinned input and disposable output directories. */
export async function extractDictionaryArchive(archivePath, csvPath, { archiveSha256, csvSha256 }) {
  if (await fileDigest(archivePath) !== archiveSha256) throw new Error("ECDICT archive checksum mismatch");
  try {
    if (await fileDigest(csvPath) === csvSha256) return csvPath;
  } catch (error) { if (error.code !== "ENOENT") throw error; }
  await mkdir(dirname(csvPath), { recursive: true });
  const temporary = await mkdtemp(resolve(dirname(csvPath), "stardict-extract-"));
  const module = await SevenZip({
    print() {}, printErr() {}, stdin: () => null,
    quit(code) { if (code !== 0) throw new Error(`7-Zip extraction failed (${code})`); },
  });
  try {
    module.FS.mkdir("/input");
    module.FS.mount(module.NODEFS, { root: dirname(archivePath) }, "/input");
    module.FS.mkdir("/output");
    module.FS.mount(module.NODEFS, { root: temporary }, "/output");
    // Extract exactly the known flat CSV member, never archive-supplied paths or executables.
    module.callMain(["e", `/input/${basename(archivePath)}`, "stardict.csv", "-o/output", "-y"]);
    const extracted = resolve(temporary, "stardict.csv");
    if (await fileDigest(extracted) !== csvSha256) throw new Error("ECDICT expanded CSV checksum mismatch");
    await rename(extracted, csvPath);
    return csvPath;
  } finally {
    module.FS.unmount("/output");
    module.FS.unmount("/input");
    await rm(temporary, { recursive: true, force: true });
  }
}
