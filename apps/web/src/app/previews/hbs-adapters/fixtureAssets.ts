import type { ExampleAssets } from "./noteExternalChanges";

/** Fixed synthetic inputs for the renderer gateway; no user files or paths. */
export const fixtureAssets: ExampleAssets = {
  image: {id: "hbs-fixture-image", displayName: "HBS 合成图片.png", mimeType: "image/png"},
  file: {id: "hbs-fixture-file", displayName: "HBS 合成附件.txt", mimeType: "text/plain"},
};
export function readFixtureAsset(id: string): number[] {
  if (id === fixtureAssets.image.id) return [...atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aGN8AAAAASUVORK5CYII=")].map(value => value.charCodeAt(0));
  if (id === fixtureAssets.file.id) return [...new TextEncoder().encode("Synthetic HBS attachment\n")];
  throw new Error(`MODEL_ASSET_NOT_FOUND：预览附件 ${id} 不存在。`);
}
