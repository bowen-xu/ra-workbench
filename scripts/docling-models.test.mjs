import { describe, it, expect } from "vitest";
import {
  mkdtemp,
  mkdir,
  writeFile,
  readFile,
  rm,
  symlink,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import {
  REQUIRED_MODELS,
  publishModels,
  modelBundleName,
} from "./docling-models.mjs";

describe("independent Docling model publication", () => {
  it("publishes immutable resources without changing application artifacts", async () => {
    const root = await mkdtemp(resolve(tmpdir(), "docling-model-release-"));
    try {
      const source = resolve(root, "source");
      for (const path of [...REQUIRED_MODELS, "licenses/NOTICE"]) {
        await mkdir(resolve(source, path, ".."), { recursive: true });
        await writeFile(resolve(source, path), `fixture:${path}`);
      }
      await mkdir(resolve(root, "dist"));
      await writeFile(resolve(root, "dist/latest.json"), "app-manifest");
      const first = await publishModels({ root, source, version: "test-1" });
      expect(first.manifest.precision).toBe("fp32");
      expect(first.name).toBe(modelBundleName(first.manifest.files));
      expect(await readFile(resolve(root, "dist/latest.json"), "utf8")).toBe(
        "app-manifest",
      );
      await writeFile(resolve(source, REQUIRED_MODELS[0]), "updated fixture");
      const second = await publishModels({ root, source, version: "test-2" });
      expect(first.name).not.toBe(second.name);
      expect(
        JSON.parse(
          await readFile(
            resolve(root, ".release/models/docling/latest.json"),
            "utf8",
          ),
        ).version,
      ).toBe("test-2");
      expect(
        await readFile(resolve(first.directory, REQUIRED_MODELS[0]), "utf8"),
      ).toContain("fixture:");
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
  it("rejects missing models and source symlinks escaping the selected resource directory", async () => {
    const root = await mkdtemp(resolve(tmpdir(), "docling-model-invalid-"));
    try {
      await mkdir(resolve(root, "licenses"));
      await writeFile(resolve(root, "licenses/NOTICE"), "notice");
      await expect(
        publishModels({ root, source: root, version: "test" }),
      ).rejects.toThrow();
      await writeFile(resolve(root, "../outside-model.onnx"), "outside");
      await symlink(
        resolve(root, "../outside-model.onnx"),
        resolve(root, REQUIRED_MODELS[0]),
      );
      await expect(
        publishModels({ root, source: root, version: "test" }),
      ).rejects.toThrow("escapes");
      await rm(resolve(root, "../outside-model.onnx"));
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
