import { spawn } from "node:child_process";
import { mkdir, stat, writeFile, rm } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import {
  ENGINE_VERSION,
  ENGINE_COMMIT,
  REQUIRED_MODELS,
  hashModel,
} from "./docling-models.mjs";

// Download only FP32 inference assets. Publishing remains a separate, explicit step.
const base =
  "https://github.com/docling-project/docling.rs/releases/download/models-v1";
const licenses = {
  "APACHE-2.0.txt":
    "https://raw.githubusercontent.com/RapidAI/RapidOCR/main/LICENSE",
  "HERON-MODEL-CARD.md":
    "https://huggingface.co/docling-project/docling-layout-heron/resolve/main/README.md",
  "TABLEFORMER-MODEL-CARD.md":
    "https://huggingface.co/docling-project/docling-models/resolve/main/README.md",
};
function download(url, path, { optional = false } = {}) {
  return new Promise((accept, reject) => {
    const child = spawn(
      "curl",
      [
        "--fail",
        "--location",
        "--silent",
        "--show-error",
        "--retry",
        "2",
        "--retry-all-errors",
        "--connect-timeout",
        "15",
        "--max-time",
        "900",
        "--write-out",
        "%{http_code}",
        url,
        "--output",
        path,
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    let status = "";
    child.stdout.on("data", (bytes) => (status += bytes));
    child.once("error", reject);
    child.once("exit", (code) =>
      code === 0
        ? accept(true)
        : optional && ["404", "410"].includes(status.trim())
          ? accept(false)
          : reject(
              new Error(
                `Model download failed (${code}, HTTP ${status}): ${url}`,
              ),
            ),
    );
  });
}
export async function fetchDoclingModels(
  directory = resolve(".tmp/docling-fp32-source"),
  { force = false } = {},
) {
  const sources = [];
  for (const path of REQUIRED_MODELS) {
    await mkdir(dirname(resolve(directory, path)), { recursive: true });
    let url = `${base}/${path.startsWith("tableformer/") ? path.slice(12) : path}`;
    if (path === "ocr_rec_v6_dict.txt")
      url =
        "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/paddle/PP-OCRv6/rec/PP-OCRv6_rec_small/ppocrv6_dict.txt";
    if (path === "ocr_rec_v6.onnx")
      url =
        "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/onnx/PP-OCRv6/rec/PP-OCRv6_rec_small.onnx";
    if (path === "ocr_det.onnx")
      url =
        "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/onnx/PP-OCRv6/det/PP-OCRv6_det_small.onnx";
    if (path === "ocr_textline_ori.onnx")
      url =
        "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.9.2/onnx/PP-OCRv5/cls/ch_PP-LCNet_x0_25_textline_ori_cls_mobile.onnx";
    const target = resolve(directory, path);
    console.log(`Fetching FP32 resource: ${path}`);
    const existing = await stat(target).catch(() => undefined);
    const { rename } = await import("node:fs/promises");
    if (force || !existing?.size) {
      await download(url, `${target}.download`);
      await rename(`${target}.download`, target);
    }
    if ((await stat(target)).size === 0)
      throw new Error(`Empty model resource: ${path}`);
    sources.push({ path, url, ...(await hashModel(target)) });
    if (path.startsWith("tableformer/") && path.endsWith(".onnx")) {
      const data = `${target}.data`;
      const existingData = await stat(data).catch(() => undefined);
      if (!force && existingData?.size) {
        sources.push({
          path: `${path}.data`,
          url: `${url}.data`,
          ...(await hashModel(data)),
        });
      } else if (
        await download(`${url}.data`, `${data}.download`, { optional: true })
      ) {
        await rename(`${data}.download`, data);
        sources.push({
          path: `${path}.data`,
          url: `${url}.data`,
          ...(await hashModel(data)),
        });
      } else {
        await rm(`${data}.download`, { force: true });
      }
    }
  }
  await mkdir(resolve(directory, "licenses"), { recursive: true });
  for (const [name, url] of Object.entries(licenses)) {
    await download(url, resolve(directory, "licenses", name));
  }
  await writeFile(
    resolve(directory, "licenses/SOURCES.json"),
    JSON.stringify(
      {
        engineVersion: ENGINE_VERSION,
        engineCommit: ENGINE_COMMIT,
        models: sources,
      },
      null,
      2,
    ),
  );
  console.log(
    `FP32 source ready: ${directory}\nPublish with pnpm models:prepare -- --source ${directory} --version <model-version>`,
  );
  return directory;
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  try {
    await fetchDoclingModels(undefined, {
      force: process.argv.includes("--force"),
    });
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
