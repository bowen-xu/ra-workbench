import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const root = resolve(scriptDirectory, "..");
const logoPath = resolve(root, "apps/web/src/assets/logo.svg");
const iconDirectory = resolve(root, "apps/desktop/src-tauri/icons");
const backgroundColor = "#ffffff";
const foregroundColor = "#111111";
const logoCenterX = 633.5;
const logoCenterY = 617;
const logoScale = 0.7;
const expectedViewBox = 'viewBox="233.5 217 800 800"';
const temporaryDirectory = await mkdtemp(
  resolve(tmpdir(), "research-assistant-icons-"),
);

try {
  const logo = await readFile(logoPath, "utf8");

  if (!logo.includes(expectedViewBox)) {
    throw new Error(
      `Expected the cropped application logo to contain ${expectedViewBox}`,
    );
  }

  const appIcon = logo
    .replace(
      "</desc>",
      `</desc>\n\t<rect x="233.5" y="217" width="800" height="800" rx="176" fill="${backgroundColor}"/>`,
    )
    .replace(
      "\t<path ",
      `\t<g transform="translate(${logoCenterX} ${logoCenterY}) scale(${logoScale}) translate(${-logoCenterX} ${-logoCenterY})">\n\t\t<path `,
    )
    .replace('fill="currentColor"', `fill="${foregroundColor}"`)
    .replace("\n</svg>", "\n\t</g>\n</svg>");
  const temporaryIcon = resolve(temporaryDirectory, "app-icon.svg");
  await writeFile(temporaryIcon, appIcon);

  const generation = spawnSync(
    "pnpm",
    [
      "--filter",
      "@research-assistant/desktop",
      "tauri",
      "icon",
      temporaryIcon,
      "--output",
      iconDirectory,
      "--ios-color",
      backgroundColor,
    ],
    { cwd: root, stdio: "inherit" },
  );

  if (generation.status !== 0) {
    throw new Error("Tauri application icon generation failed");
  }
} finally {
  await rm(temporaryDirectory, { recursive: true, force: true });
}
