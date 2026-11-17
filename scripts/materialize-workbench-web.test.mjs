import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import {createHash} from "node:crypto";
import {afterEach, beforeEach, expect, it} from "vitest";
import {materializeWorkbenchWeb} from "./materialize-workbench-web.mjs";

let root, receipt, distribution;
beforeEach(async () => {
  root = await fs.mkdtemp(path.join(os.tmpdir(), "workbench-web-materialize-"));
  const output = path.join(root, "package");
  await fs.mkdir(path.join(output, "web/assets"), {recursive: true});
  const artifacts = [];
  for (const [file, bytes] of [["index.html", "original"], ["assets/main.js", "synthetic-js"]]) {
    const target = path.join(output, "web", file);
    await fs.writeFile(target, bytes);
    const stat = await fs.stat(target);
    artifacts.push({file: "web/" + file, bytes: stat.size, mode: stat.mode & 0o777,
      sha256: createHash("sha256").update(bytes).digest("hex")});
  }
  receipt = {output, artifacts};
  distribution = path.join(root, "apps/web/dist");
});
afterEach(async () => fs.rm(root, {recursive: true, force: true}));

it("preserves exact asset inodes and timestamps on repeated materialization", async () => {
  expect(await materializeWorkbenchWeb({root, receipt})).toEqual({reused: false});
  const file = path.join(distribution, "index.html");
  const before = await fs.stat(file, {bigint: true});
  expect(await materializeWorkbenchWeb({root, receipt})).toEqual({reused: true});
  const after = await fs.stat(file, {bigint: true});
  expect([after.ino, after.mtimeNs]).toEqual([before.ino, before.mtimeNs]);
});

it.each(["missing", "corrupt", "extra", "symlink", "mode"])("repairs a %s distribution from the verified package", async kind => {
  await materializeWorkbenchWeb({root, receipt});
  const file = path.join(distribution, "index.html");
  if (kind === "missing") await fs.rm(file);
  if (kind === "corrupt") await fs.writeFile(file, "modified"); // Same byte length.
  if (kind === "extra") await fs.writeFile(path.join(distribution, "extra.js"), "unexpected");
  if (kind === "symlink") {
    await fs.rm(file);
    await fs.symlink(path.join(receipt.output, "web/index.html"), file);
  }
  if (kind === "mode") await fs.chmod(file, 0o700);
  expect(await materializeWorkbenchWeb({root, receipt})).toEqual({reused: false});
  expect(await fs.readFile(file, "utf8")).toBe("original");
  expect((await fs.lstat(file)).isFile()).toBe(true);
  expect(await materializeWorkbenchWeb({root, receipt})).toEqual({reused: true});
});

it("keeps the previous distribution when staging fails validation", async () => {
  await materializeWorkbenchWeb({root, receipt});
  const file = path.join(distribution, "index.html");
  await fs.writeFile(file, "previous valid build");
  await fs.writeFile(path.join(receipt.output, "web/index.html"), "corrupt source");
  await expect(materializeWorkbenchWeb({root, receipt})).rejects.toThrow("workbench_web_distribution_invalid");
  expect(await fs.readFile(file, "utf8")).toBe("previous valid build");
});
