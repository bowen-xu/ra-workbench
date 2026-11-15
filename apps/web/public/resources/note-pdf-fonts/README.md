# Local PDF font resources

These resources are loaded only during PDF export. They are not fetched from a
CDN and are not used as application UI fonts.

- `ResearchAssistantSans-Regular.ttf` and `ResearchAssistantSans-Bold.ttf` are
  renamed static instances (wght 400 / 700) of
  `Sans/Variable/TTF/Subset/NotoSansSC-VF.ttf` from
  https://github.com/notofonts/noto-cjk at
  `f8d157532fbfaeda587e826d4cd5b21a49186f7c`. License: SIL OFL 1.1,
  retained in `OFL.txt`. Original copyright/name records are retained except
  family/style/unique/full/PostScript/preferred-family names (IDs 1, 2, 3, 4,
  6, 16, 17), renamed to Research Assistant Sans to avoid reserved font names.
- Generated using fontTools 4.60.1 `instantiateVariableFont` with
  `{wght: 400}` / `{wght: 700}`, then saved as TrueType. Source weights are
  instanced rather than subsetting characters: Chinese coverage remains
  available offline for arbitrary notes. PDF generation embeds only used glyphs.
- `KaTeX_*.ttf` are unmodified files from the installed, pinned `katex` 0.18.1
  package's `dist/fonts` directory. The font files' embedded name records
  specify OFL 1.1 and reserved font names, retained with their copyright notices
  in `KaTeX-OFL.txt`. The package's MIT notice is additionally retained in
  `KaTeX-LICENSE.txt`. They preserve mathematical glyphs and font metrics.

Generation/check commands run through the project's `research` conda environment;
fontTools is a generation tool, not an application runtime dependency.
The two font instances add approximately 20 MiB of bundled resources. They are
read through fixed native asset names or same-origin browser URLs only, and are
released after each export.
