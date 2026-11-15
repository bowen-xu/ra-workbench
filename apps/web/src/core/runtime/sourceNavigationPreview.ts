export const SOURCE_NAVIGATION_PREVIEW_URL = "/?preview=app&demo=source-navigation";
export function isSourceNavigationPreview(search: string): boolean {
  const params = new URLSearchParams(search);
  return params.get("preview") === "app" && params.get("demo") === "source-navigation";
}
