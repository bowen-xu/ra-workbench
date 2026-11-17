import fs from "node:fs/promises";
import path from "node:path";
import {createHash} from "node:crypto";

async function matches(directory, expected) {
  try {
    if (!(await fs.lstat(directory)).isDirectory()) return false;
    const remaining = new Map(expected.map(entry => [entry.file, entry]));
    async function visit(relative) {
      for (const entry of await fs.readdir(path.join(directory, relative), {withFileTypes: true})) {
        const file = path.posix.join(relative, entry.name);
        if (entry.isDirectory()) {
          if (!await visit(file)) return false;
        } else {
          const target = path.join(directory, file);
          const stat = await fs.lstat(target);
          const artifact = remaining.get(file);
          if (!stat.isFile() || !artifact || stat.size !== artifact.bytes
            || (stat.mode & 0o777) !== artifact.mode) return false;
          const sha256 = createHash("sha256").update(await fs.readFile(target)).digest("hex");
          if (sha256 !== artifact.sha256) return false;
          remaining.delete(file);
        }
      }
      return true;
    }
    return await visit("") && remaining.size === 0;
  } catch (error) {
    if (["ENOENT", "ENOTDIR"].includes(error.code)) return false;
    throw error;
  }
}

/** Preserve matching asset timestamps. Verify a full stage before replacing dist. */
export async function materializeWorkbenchWeb({root, receipt}) {
  const expected = receipt.artifacts.filter(entry => entry.file.startsWith("web/"))
    .map(entry => ({...entry, file: entry.file.slice(4)}));
  if (!expected.length) throw new Error("workbench_web_artifacts_missing");
  const distribution = path.join(root, "apps/web/dist");
  if (await matches(distribution, expected)) {
    console.log("[desktop] Reusing verified frontend distribution");
    return {reused: true};
  }
  const work = path.join(root, ".cache/work");
  await fs.mkdir(work, {recursive: true});
  await fs.mkdir(path.dirname(distribution), {recursive: true});
  const staging = await fs.mkdtemp(path.join(work, "web-distribution-"));
  const prepared = path.join(staging, "prepared");
  const previous = path.join(staging, "previous");
  let backedUp = false;
  try {
    await fs.cp(path.join(receipt.output, "web"), prepared, {recursive: true});
    if (!await matches(prepared, expected)) throw new Error("workbench_web_distribution_invalid");
    try {
      await fs.rename(distribution, previous);
      backedUp = true;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    try {
      await fs.rename(prepared, distribution);
    } catch (error) {
      // A concurrent publication may already own dist. Preserve that complete tree.
      if (!await matches(distribution, expected)) {
        if (backedUp) {
          await fs.rename(previous, distribution).catch(restoreError => {
            throw new AggregateError([error, restoreError], `workbench_web_restore_failed: retained ${previous}`);
          });
          backedUp = false;
        }
        throw error;
      }
    }
    console.log("[desktop] Published verified frontend distribution");
    return {reused: false};
  } finally {
    // A failed restore keeps its previous tree available for recovery.
    if (!backedUp || await matches(distribution, expected)) await fs.rm(staging, {recursive: true, force: true});
  }
}
