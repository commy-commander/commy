import assert from "node:assert/strict";
import { test } from "node:test";
import { checksumsUrl, detectPlatform, downloadsFrom, formatSize, orderDownloads } from "../lib/release-assets.ts";

const asset = (name, size = 1000) => ({ name, size, browser_download_url: `https://dl.test/${name}` });
const ASSETS = [
  asset("Commy_0.2.0_x64-setup.exe", 12_000_000),
  asset("Commy_0.2.0_x64-portable.zip"),
  asset("Commy_0.2.0_universal.dmg"),
  asset("Commy_0.2.0_universal.app.tar.gz"),
  asset("Commy_0.2.0_amd64.deb"),
  asset("Commy_0.2.0_amd64.AppImage"),
  asset("SHA256SUMS.txt"),
  asset("evil_x64-setup.exe"),
];

test("maps every release asset to a platform", () => {
  const d = downloadsFrom(ASSETS);
  assert.equal(d.length, 6);
  assert.deepEqual(d.map((x) => x.platform), ["windows", "windows", "macos", "macos", "linux", "linux"]);
  assert.equal(d.filter((x) => x.primary).length, 3);
  assert.ok(d.every((x) => x.url.includes("/Commy_")), "only Commy_ assets are offered");
  assert.equal(checksumsUrl(ASSETS), "https://dl.test/SHA256SUMS.txt");
});

test("missing assets are skipped, not invented", () => {
  assert.deepEqual(downloadsFrom([]), []);
  assert.equal(downloadsFrom([asset("Commy_1.0.0_universal.dmg")]).length, 1);
  assert.equal(checksumsUrl([]), null);
});

test("detects desktop platforms from user agents", () => {
  assert.equal(detectPlatform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), "windows");
  assert.equal(detectPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)"), "macos");
  assert.equal(detectPlatform("Mozilla/5.0 (X11; Linux x86_64)"), "linux");
  assert.equal(detectPlatform("Mozilla/5.0 (Linux; Android 14)"), null);
  assert.equal(detectPlatform("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X)"), null);
  assert.equal(detectPlatform(null), null);
});

test("orders the visitor's platform first, primary package first", () => {
  const d = orderDownloads(downloadsFrom(ASSETS), "macos");
  assert.deepEqual(d.slice(0, 2).map((x) => x.label), ["macOS", "macOS app archive"]);
  assert.equal(orderDownloads(downloadsFrom(ASSETS), null)[0].label, "Windows installer");
});

test("formats sizes", () => {
  assert.equal(formatSize(12_000_000), "11.4 MB");
  assert.equal(formatSize(300_000_000), "286 MB");
  assert.equal(formatSize(2048), "2 KB");
  assert.equal(formatSize(0), "");
});
