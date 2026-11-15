import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import SevenZip from "7z-wasm";
import { extractDictionaryArchive } from "./extract-dictionary-archive.mjs";

const digest = bytes => createHash("sha256").update(bytes).digest("hex");

describe("pinned stardict archive extraction", () => {
  it("extracts only the expected CSV, verifies both hashes and repairs a damaged cache", async () => {
    const root = await mkdtemp(join(tmpdir(), "stardict-archive-"));
    try {
      const content = "word,translation\ncognitive architecture,认知架构\n";
      await writeFile(join(root, "stardict.csv"), content); await writeFile(join(root, "unwanted.txt"), "not a dictionary");
      const sevenZip = await SevenZip({ print() {}, printErr() {} });
      sevenZip.FS.mkdir("/fixture"); sevenZip.FS.mount(sevenZip.NODEFS, { root }, "/fixture");
      sevenZip.FS.chdir("/fixture"); sevenZip.callMain(["a", "-t7z", "fixture.7z", "stardict.csv", "unwanted.txt"]);
      sevenZip.FS.chdir("/"); sevenZip.FS.unmount("/fixture");
      const archive = join(root, "fixture.7z"), destination = join(root, "cache", "stardict.csv");
      const hashes = { archiveSha256: digest(await readFile(archive)), csvSha256: digest(content) };
      expect(await extractDictionaryArchive(archive, destination, hashes)).toBe(destination);
      expect(await readFile(destination, "utf8")).toBe(content);
      expect(await readdir(join(root, "cache"))).toEqual(["stardict.csv"]);
      await writeFile(destination, "damaged");
      await extractDictionaryArchive(archive, destination, hashes);
      expect(await readFile(destination, "utf8")).toBe(content);
      await expect(extractDictionaryArchive(archive, destination, { ...hashes, archiveSha256: "wrong" })).rejects.toThrow("archive checksum");
      await expect(extractDictionaryArchive(archive, destination, { ...hashes, csvSha256: "wrong" })).rejects.toThrow("CSV checksum");
      expect(await readdir(join(root, "cache"))).toEqual(["stardict.csv"]);
    } finally { await rm(root, { recursive: true, force: true }); }
  });
});
