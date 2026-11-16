import {copyFileSync, readFileSync} from "node:fs";
import {createHash} from "node:crypto";
import {fileURLToPath} from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../../../ra-workbench-sdk/.tmp/artifacts/", import.meta.url));

export function copySdkArtifact(name, destination) {
  const manifest = JSON.parse(readFileSync(path.join(root, "sdk-artifacts.json"), "utf8"));
  const artifact = manifest.artifacts.find(value => value.kind === "web" && value.name === "@research-assistant/" + name);
  if (!artifact) throw new Error("SDK_TEST_ARTIFACT_MISSING: " + name);
  const source = path.join(root, artifact.file);
  if (createHash("sha256").update(readFileSync(source)).digest("hex") !== artifact.sha256) {
    throw new Error("SDK_TEST_ARTIFACT_CHANGED: " + name);
  }
  copyFileSync(source, destination);
}
