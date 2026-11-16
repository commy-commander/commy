import { createHash } from "node:crypto";
import { posix } from "node:path";

export const ASSETS_DIR = "assets";

const LINK_TAG = /<link rel="stylesheet"[^>]*\/>/g;
const SCRIPT_BLOCK = /(?:<script src="[^"]+"><\/script>\s*)+/g;
const SCRIPT_SRC = /<script src="([^"]+)"><\/script>/g;
const CSS_URL_OR_STRING = /\/\*[\s\S]*?\*\/|url\(\s*(["'])(.*?)\1\s*\)|url\(\s*([^"'()\s]+)\s*\)|"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'/g;
const RELATIVE_IMPORT = /\bimport\(\s*(["'])(\.\.?\/[^"']+)\1\s*\)/g;
const LEADING_STRICT = /^(?:﻿)?(?:\s+|\/\/[^\n]*\n|\/\*[\s\S]*?\*\/)*(["'])use strict\1/;

export function contentHash(bytes) {
  return createHash("sha256").update(bytes).digest("hex").slice(0, 16);
}

export function hashedName(path, bytes) {
  const { name, ext } = posix.parse(path);
  return `${ASSETS_DIR}/${name}-${contentHash(bytes)}${ext}`;
}

export function scriptIsStrict(code) {
  return LEADING_STRICT.test(code);
}

export function strictnessRuns(scripts) {
  const runs = [];
  for (const script of scripts) {
    const strict = scriptIsStrict(script.code);
    const last = runs.at(-1);
    if (last && last.strict === strict) last.scripts.push(script);
    else runs.push({ strict, scripts: [script] });
  }
  return runs;
}

export function joinScripts(codes) {
  return codes.map(code => `${code.replace(/^﻿/, "")}\n;\n`).join("");
}

function attribute(tag, name) {
  const match = new RegExp(`\\s${name}="([^"]*)"`).exec(tag);
  return match ? match[1] : null;
}

function isExternal(url) {
  return /^(?:[a-z][a-z0-9+.-]*:|\/|#)/i.test(url);
}

export function stylesheetGroups(html) {
  const groups = [];
  let previousEnd = -1;
  for (const match of html.matchAll(LINK_TAG)) {
    const tag = match[0];
    const link = { tag, start: match.index, end: match.index + tag.length, href: attribute(tag, "href"), id: attribute(tag, "id"), media: attribute(tag, "media") };
    if (!link.href || isExternal(link.href)) {
      previousEnd = -1;
      continue;
    }
    const last = groups.at(-1);
    const adjacent = last && previousEnd >= 0 && /^\s*$/.test(html.slice(previousEnd, link.start));
    if (adjacent && last.media === link.media && Boolean(last.id) === Boolean(link.id)) last.links.push(link);
    else groups.push({ id: link.id, media: link.media, links: [link] });
    previousEnd = link.end;
  }
  return groups;
}

function stylesheetTag({ id, media }, href) {
  return `<link rel="stylesheet"${id ? ` id="${id}"` : ""} href="${href}"${media ? ` media="${media}"` : ""} />`;
}

function replaceRanges(html, replacements) {
  let out = "";
  let at = 0;
  for (const { start, end, text } of replacements.sort((a, b) => a.start - b.start)) {
    out += html.slice(at, start) + text;
    at = end;
  }
  return out + html.slice(at);
}

export function createAssetStore({ read, write }) {
  const moved = new Map();
  return {
    moved,
    put(path, bytes) {
      const name = hashedName(path, bytes);
      write(name, bytes);
      return name;
    },
    hashFile(path) {
      if (!moved.has(path)) moved.set(path, this.put(path, read(path)));
      return moved.get(path);
    },
  };
}

export function rewriteCssUrls(css, sheetPath, assets) {
  return css.replace(CSS_URL_OR_STRING, (whole, quoted, inQuotes, bare) => {
    const url = inQuotes ?? bare;
    const quote = quoted ?? "";
    if (url === undefined || isExternal(url) || url.startsWith("data:")) return whole;
    const [path, suffix = ""] = url.split(/(?=[?#])/);
    const target = posix.normalize(posix.join(posix.dirname(sheetPath), path));
    const hashed = assets.hashFile(target);
    return `url(${quote}${posix.relative(ASSETS_DIR, hashed)}${suffix}${quote})`;
  });
}

export function rewriteRelativeImports(code, scriptPath, moved) {
  return code.replace(RELATIVE_IMPORT, (whole, quote, specifier) => {
    const target = posix.normalize(posix.join(posix.dirname(scriptPath), specifier));
    const hashed = moved.get(target);
    if (!hashed) throw new Error(`${scriptPath} imports ${specifier}, which the web bundle does not hash`);
    return `import(${quote}./${posix.relative(ASSETS_DIR, hashed)}${quote})`;
  });
}

export function hashWasmPackage(jsPath, wasmPath, assets, { read, remove }) {
  const wasmName = assets.put(wasmPath, read(wasmPath));
  const wasmFile = posix.basename(wasmPath);
  const js = read(jsPath).toString("utf8");
  if (!js.includes(`'${wasmFile}'`)) throw new Error(`${jsPath} no longer names '${wasmFile}'`);
  const jsName = assets.put(jsPath, Buffer.from(js.replaceAll(`'${wasmFile}'`, `'${posix.basename(wasmName)}'`)));
  assets.moved.set(jsPath, jsName);
  assets.moved.set(wasmPath, wasmName);
  remove(jsPath);
  remove(wasmPath);
  return { js: jsName, wasm: wasmName };
}

export function bundleStylesheets(html, assets, read) {
  const replacements = stylesheetGroups(html).map(group => {
    const css = group.links.map(link => rewriteCssUrls(read(link.href).toString("utf8"), link.href, assets)).join("\n");
    const href = assets.put(group.links[0].href, Buffer.from(css));
    return { start: group.links[0].start, end: group.links.at(-1).end, text: stylesheetTag(group, href) };
  });
  return replaceRanges(html, replacements);
}

export function bundleScripts(html, assets, read) {
  const replacements = [...html.matchAll(SCRIPT_BLOCK)].map(match => {
    const block = match[0];
    const scripts = [...block.matchAll(SCRIPT_SRC)].map(([, src]) => {
      if (isExternal(src)) throw new Error(`the web page loads ${src}, which the web bundle cannot hold`);
      return { src, code: rewriteRelativeImports(read(src).toString("utf8"), src, assets.moved) };
    });
    const trailing = /\s*$/.exec(block)[0];
    const tags = strictnessRuns(scripts).map(run => {
      const name = assets.put(`scripts/${run.strict ? "strict" : "scripts"}.js`, Buffer.from(joinScripts(run.scripts.map(script => script.code))));
      return `<script src="${name}"></script>`;
    });
    return { start: match.index, end: match.index + block.length, text: tags.join("\n  ") + trailing };
  });
  return replaceRanges(html, replacements);
}

export function preloadTags({ js, wasm }) {
  return [`<link rel="modulepreload" href="${js}" />`, `<link rel="preload" href="${wasm}" as="fetch" type="application/wasm" crossorigin />`];
}

export function bundleWebPage(html, { read, write, remove, wasmPackage }) {
  const assets = createAssetStore({ read, write });
  const pkg = hashWasmPackage(wasmPackage.js, wasmPackage.wasm, assets, { read, remove });
  const firstStylesheet = html.search(LINK_TAG);
  if (firstStylesheet < 0) throw new Error("the web page has no stylesheet to bundle");
  const preloaded = html.slice(0, firstStylesheet) + preloadTags(pkg).join("\n  ") + "\n  " + html.slice(firstStylesheet);
  return bundleScripts(bundleStylesheets(preloaded, assets, read), assets, read);
}
