import { REPO } from "./site";
import { checksumsUrl, downloadsFrom, type Download, type GithubAsset } from "./release-assets";

export type LatestRelease = {
  tag: string;
  name: string;
  url: string;
  publishedAt: string;
  downloads: Download[];
  checksums: string | null;
};

// Cached for ten minutes so page views never hammer the GitHub API.
export async function getLatestRelease(): Promise<LatestRelease | null> {
  const headers: Record<string, string> = { accept: "application/vnd.github+json", "user-agent": "commy-web" };
  if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers,
      next: { revalidate: 600 },
    });
    if (!res.ok) return null;
    const data = (await res.json()) as {
      tag_name: string;
      name: string | null;
      html_url: string;
      published_at: string;
      assets: GithubAsset[];
    };
    return {
      tag: data.tag_name,
      name: data.name || data.tag_name,
      url: data.html_url,
      publishedAt: data.published_at,
      downloads: downloadsFrom(data.assets ?? []),
      checksums: checksumsUrl(data.assets ?? []),
    };
  } catch {
    return null;
  }
}
