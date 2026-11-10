import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const MAP_FILE = join(HERE, "studio-icon-map.json");
const OUT_FILE = join(HERE, "../ui/vendor/phosphor/icons.js");
const REST_WEIGHT = "regular";
const ON_WEIGHT = "fill";

const assetsDir = process.argv[2];
if (!assetsDir) {
  console.error("usage: node apps/desktop/tools/studio-icons.mjs <unpacked @phosphor-icons/core>/assets");
  process.exit(1);
}

function drawing(weight, name) {
  const file = join(assetsDir, weight, weight === REST_WEIGHT ? `${name}.svg` : `${name}-${weight}.svg`);
  if (!existsSync(file)) return null;
  return readFileSync(file, "utf8").replace(/^<svg[^>]*>/, "").replace(/<\/svg>\s*$/, "");
}

const map = JSON.parse(readFileSync(MAP_FILE, "utf8"));
const paths = {};
const missing = [];
for (const [name, source] of Object.entries(map)) {
  const rest = drawing(REST_WEIGHT, source);
  const on = drawing(ON_WEIGHT, source);
  if (!rest || !on) {
    missing.push(`${name} > ${source}`);
    continue;
  }
  paths[name] = [rest, on];
}
if (missing.length) {
  console.error(`missing drawings: ${missing.join(", ")}`);
  process.exit(1);
}

const body = Object.entries(paths).map(([name, pair]) => `  ${JSON.stringify(name)}: ${JSON.stringify(pair)},`).join("\n");
writeFileSync(OUT_FILE, `const STUDIO_ICON_PATHS = {\n${body}\n};\n\nif (typeof module !== "undefined" && module.exports) {\n  module.exports = { STUDIO_ICON_PATHS };\n}\n`);
console.log(`${OUT_FILE}: ${Object.keys(paths).length} icons`);
