# Docling menu bar logo

The approved A variant derives from `apps/web/src/assets/logo.svg`: keep the
seal-script 研 glyph, remove the separate outline, and cut the glyph out of a
rounded square. The cutouts and surrounding canvas are transparent, not white.
macOS uses the PNG as a template image and supplies the foreground color.

`menu-bar-logo.svg` is the editable source. `menu-bar-logo.png` is its 72×72 RGBA
export, embedded in the daemon executable. The native status item displays it
at 18 points; no external asset lookup is required at runtime. The main desktop
application icon is unchanged.

Regenerate from the repository root using the existing Tauri icon tool:

```sh
pnpm --filter @research-assistant/desktop exec tauri icon "$PWD/apps/desktop/docling-daemon/assets/menu-bar-logo.svg" --output "$PWD/.tmp/docling-tray-icon" --png 72
cp .tmp/docling-tray-icon/72x72.png apps/desktop/docling-daemon/assets/menu-bar-logo.png
```
