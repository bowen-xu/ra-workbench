import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { buildDictionary } from "./prepare-dictionary.mjs";

describe("ECDICT asset generation", () => {
  it("uses expanded entries first, adds missing base terms and keeps explicit inflection links", async () => {
    const root = await mkdtemp(join(tmpdir(), "ecdict-expanded-"));
    try {
      const primary = join(root, "stardict.csv"), fallback = join(root, "ecdict.csv"), output = join(root, "output");
      const header = "word,phonetic,translation,definition,exchange\n";
      await writeFile(primary, header + "go,ɡəʊ,扩展释义,,p:went\ncognitive architecture,,认知架构,,\n");
      await writeFile(fallback, header + "go,,基础旧释义,,p:went\nbase-only,,基础独有释义,,\nwent,,过去式,,0:go\n");
      const manifest = await buildDictionary(primary, output, "notice", { fallbackCsvPath: fallback });
      expect(manifest.entryCount).toBe(4); expect(manifest.primaryEntryCount).toBe(2); expect(manifest.fallbackEntryCount).toBe(2);
      const record = async word => JSON.parse(await readFile(join(output,
        `${createHash("sha256").update(word).digest("hex").slice(0, 2)}.json`), "utf8"))[word];
      expect((await record("go")).e).toEqual([["go", "ɡəʊ", "扩展释义", "", "p:went"]]);
      expect((await record("cognitive architecture")).e[0][2]).toBe("认知架构");
      expect((await record("base-only")).e[0][2]).toBe("基础独有释义");
      expect((await record("went")).e).toHaveLength(1); expect((await record("went")).l).toEqual(["go"]);
    } finally { await rm(root, { recursive: true, force: true }); }
  });
  it("preserves quoted CSV definitions, multiple forms and source notices with deterministic checksums", async () => {
    const root = await mkdtemp(join(tmpdir(), "ecdict-build-"));
    try {
      const csv = join(root, "source.csv"), output = join(root, "output");
      await writeFile(csv, 'word,phonetic,translation,definition,exchange\n'
        + 'Go,ɡəʊ,"v. 去,走","move\ntravel",p:went/d:gone/3:goes\n'
        + 'went,,过去式,,0:go\n');
      const manifest = await buildDictionary(csv, output, "MIT notice");
      expect(manifest.entryCount).toBe(2);
      for (const [key, file] of Object.entries(manifest.files)) {
        const bytes = await readFile(join(output, `${key}.json`));
        expect(bytes.length).toBe(file.bytes);
        expect(createHash("sha256").update(bytes).digest("hex")).toBe(file.sha256);
      }
      const shard = async word => JSON.parse(await readFile(join(output,
        `${createHash("sha256").update(word).digest("hex").slice(0, 2)}.json`), "utf8"));
      expect((await shard("go")).go.e[0]).toEqual(["Go", "ɡəʊ", "v. 去,走", "move\ntravel", "p:went/d:gone/3:goes"]);
      expect((await shard("went")).went.l).toEqual(["go"]);
      expect((await shard("gone")).gone.l).toEqual(["go"]);
      expect(await readFile(join(output, "LICENSE.txt"), "utf8")).toBe("MIT notice");
      expect(await buildDictionary(csv, output, "MIT notice")).toEqual(manifest);
    } finally { await rm(root, { recursive: true, force: true }); }
  });
  it("rejects malformed records rather than silently dropping data", async () => {
    const root = await mkdtemp(join(tmpdir(), "ecdict-invalid-"));
    try {
      const csv = join(root, "source.csv"); await writeFile(csv, "word,translation\ncat,猫,extra\n");
      await expect(buildDictionary(csv, join(root, "output"), "notice")).rejects.toThrow();
    } finally { await rm(root, { recursive: true, force: true }); }
  });
  it("indexes strip variants from the full corpus and frequency-marked spelling candidates", async () => {
    const root = await mkdtemp(join(tmpdir(), "ecdict-suggestions-"));
    try {
      const csv = join(root,"source.csv"), output = join(root,"output");
      await writeFile(csv,"word,translation,frq,bnc,collins,oxford\nlong-time,长期,0,0,0,0\nlong time,长时间,0,0,0,0\nwork,工作,100,200,5,1\nmodel,模型,200,300,4,1\nrareunranked,罕见,0,0,0,0\n");
      const manifest = await buildDictionary(csv,output,"notice");
      const key=createHash("sha256").update("longtime").digest("hex").slice(0,2);
      const shard=JSON.parse(await readFile(join(output,`${key}.json`),"utf8"));
      expect(shard.longtime.s).toEqual(["long-time","long time"]);
      expect(manifest.suggestions.spellingWordCount).toBe(2);
      expect(JSON.parse(await readFile(join(output,"spelling-04.json"),"utf8"))).toEqual([["work",100]]);
      expect(JSON.parse(await readFile(join(output,"spelling-05.json"),"utf8"))).toEqual([["model",200]]);
      for(const [name,file] of Object.entries(manifest.suggestions.files)) {
        const bytes=await readFile(join(output,name));expect(bytes.length).toBe(file.bytes);
        expect(createHash("sha256").update(bytes).digest("hex")).toBe(file.sha256);
      }
      expect(await buildDictionary(csv,output,"notice")).toEqual(manifest);
    } finally { await rm(root,{recursive:true,force:true}); }
  });
});
