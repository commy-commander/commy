import { REPO } from "./site";
import { checksumsUrl, downloadsFrom, parseChecksums, type Download, type GithubAsset } from "./release-assets";

export type LatestRelease = {
  tag: string;
  name: string;
  url: string;
  publishedAt: string;
  downloads: Download[];
  checksums: string | null;
  // file name -> SHA-256, empty when SHA256SUMS.txt is missing or unreadable
  hashes: Record<string, string>;
};

const REVALIDATE = 600;

// Cached for ten minutes so page views never hammer the GitHub API.
// /releases/latest skips drafts and prereleases.
export async function getLatestRelease(): Promise<LatestRelease | null> {
  const headers: Record<string, string> = { accept: "application/vnd.github+json", "user-agent": "commy-web" };
  if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers,
      next: { revalidate: REVALIDATE },
    });
    if (!res.ok) return null;
    const data = (await res.json()) as {
      tag_name: string;
      name: string | null;
      html_url: string;
      published_at: string;
      assets: GithubAsset[];
    };
    const assets = data.assets ?? [];
    const checksums = checksumsUrl(assets);
    return {
      tag: data.tag_name,
      name: data.name || data.tag_name,
      url: data.html_url,
      publishedAt: data.published_at,
      downloads: downloadsFrom(assets),
      checksums,
      hashes: checksums ? await fetchHashes(checksums) : {},
    };
  } catch {
    return null;
  }
}

async function fetchHashes(url: string): Promise<Record<string, string>> {
  try {
    const res = await fetch(url, { headers: { "user-agent": "commy-web" }, next: { revalidate: REVALIDATE } });
    return res.ok ? parseChecksums(await res.text()) : {};
  } catch {
    return {};
  }
}
