import { closeSync, mkdtempSync, openSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { expect, it } from "vitest";
import { publishDoclingRuntime } from "./prepare-docling-runtime.mjs";

it("publishes a new inode without changing an already opened executable", () => {
  const root = mkdtempSync(resolve(tmpdir(), "docling-publication-"));
  const source = resolve(root, "source");
  const destination = resolve(root, "resources/daemon");
  let previous;
  try {
    writeFileSync(source, "first");
    publishDoclingRuntime(source, destination);
    previous = openSync(destination, "r");
    writeFileSync(source, "second");
    publishDoclingRuntime(source, destination);
    expect(readFileSync(destination, "utf8")).toBe("second");
    expect(readFileSync(previous, "utf8")).toBe("first");
  } finally {
    if (previous !== undefined) closeSync(previous);
    rmSync(root, { recursive: true, force: true });
  }
});

it("leaves an unchanged server file untouched when preparing the desktop bundle", () => {
  const root = mkdtempSync(resolve(tmpdir(), "docling-unchanged-"));
  try {
    const source = resolve(root, "source");
    const destination = resolve(root, "resources/daemon");
    writeFileSync(source, "same-server");
    publishDoclingRuntime(source, destination);
    const previous = statSync(destination);
    publishDoclingRuntime(source, destination);
    const current = statSync(destination);
    expect(current.ino).toBe(previous.ino);
    expect(current.mtimeMs).toBe(previous.mtimeMs);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
