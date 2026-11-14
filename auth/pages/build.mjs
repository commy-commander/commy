import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, "src");
const out = join(here, "dist");

const shared = {
  bundle: true,
  jsx: "automatic",
  jsxImportSource: "preact",
  alias: { react: "preact/compat", "react-dom": "preact/compat", "react/jsx-runtime": "preact/jsx-runtime" },
  legalComments: "none",
  logLevel: "warning",
};

async function contentHash(file) {
  return createHash("sha256").update(await readFile(file)).digest("hex").slice(0, 16);
}

await rm(out, { recursive: true, force: true });
await mkdir(out, { recursive: true });

await Promise.all([
  build({ ...shared, entryPoints: [join(src, "client.jsx")], outfile: join(out, "client.js"), format: "iife", platform: "browser", target: "es2022", minify: true }),
  build({ ...shared, entryPoints: [join(src, "server.jsx")], outfile: join(out, "server.js"), format: "esm", platform: "node", target: "node24" }),
  build({ entryPoints: [join(src, "pages.css")], outfile: join(out, "pages.css"), bundle: true, minify: true, logLevel: "warning", loader: { ".woff2": "file" }, assetNames: "[name]-[hash]" }),
]);

await copyFile(join(src, "logo.svg"), join(out, "logo.svg"));

const VERSIONED = ["client.js", "pages.css", "logo.svg"];
const assets = Object.fromEntries(await Promise.all(VERSIONED.map(async (name) => [name, await contentHash(join(out, name))])));
await writeFile(join(out, "assets.json"), JSON.stringify(assets));
