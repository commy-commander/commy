// Maps GitHub release assets to the platforms the downloads page offers.
// Names follow .github/workflows/release.yml: Commy_<version>_<suffix>.

export type Platform = "windows" | "macos" | "linux";

export type GithubAsset = { name: string; browser_download_url: string; size: number };

export type Download = {
  platform: Platform;
  label: string;
  kind: string;
  url: string;
  size: number;
  primary: boolean;
};

const PATTERNS: { suffix: string; platform: Platform; label: string; kind: string; primary: boolean }[] = [
  { suffix: "_x64-setup.exe", platform: "windows", label: "Windows installer", kind: "64-bit .exe", primary: true },
  { suffix: "_x64-portable.zip", platform: "windows", label: "Windows portable", kind: "64-bit .zip, no install", primary: false },
  { suffix: "_universal.dmg", platform: "macos", label: "macOS", kind: "Apple silicon and Intel .dmg", primary: true },
  { suffix: "_universal.app.tar.gz", platform: "macos", label: "macOS app archive", kind: ".app in .tar.gz", primary: false },
  { suffix: "_amd64.AppImage", platform: "linux", label: "Linux AppImage", kind: "64-bit, any distro", primary: true },
  { suffix: "_amd64.deb", platform: "linux", label: "Debian and Ubuntu", kind: "64-bit .deb", primary: false },
];

export const PLATFORM_NAMES: Record<Platform, string> = { windows: "Windows", macos: "macOS", linux: "Linux" };

export function downloadsFrom(assets: GithubAsset[]): Download[] {
  const out: Download[] = [];
  for (const p of PATTERNS) {
    const asset = assets.find((a) => a.name.startsWith("Commy_") && a.name.endsWith(p.suffix));
    if (asset) {
      out.push({ platform: p.platform, label: p.label, kind: p.kind, url: asset.browser_download_url, size: asset.size, primary: p.primary });
    }
  }
  return out;
}

export function checksumsUrl(assets: GithubAsset[]): string | null {
  return assets.find((a) => a.name === "SHA256SUMS.txt")?.browser_download_url ?? null;
}

// Best guess from the User-Agent; null when unsure (phones, bots).
export function detectPlatform(userAgent: string | null | undefined): Platform | null {
  const ua = (userAgent ?? "").toLowerCase();
  if (/iphone|ipad|android/.test(ua)) return null;
  if (ua.includes("windows")) return "windows";
  if (ua.includes("mac os x") || ua.includes("macintosh")) return "macos";
  if (ua.includes("linux") || ua.includes("x11")) return "linux";
  return null;
}

// Recommended platform first, primary package before alternatives.
export function orderDownloads(downloads: Download[], preferred: Platform | null): Download[] {
  const rank = (d: Download) => (d.platform === preferred ? 0 : 1) * 10 + (["windows", "macos", "linux"].indexOf(d.platform) * 2) + (d.primary ? 0 : 1);
  return [...downloads].sort((a, b) => rank(a) - rank(b));
}

export function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const mb = bytes / (1024 * 1024);
  return mb >= 1 ? `${mb.toFixed(mb >= 100 ? 0 : 1)} MB` : `${Math.ceil(bytes / 1024)} KB`;
}
