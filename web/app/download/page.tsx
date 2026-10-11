import type { Metadata } from "next";
import Link from "next/link";
import { headers } from "next/headers";
import { getLatestRelease } from "@/lib/releases";
import { detectPlatform, formatSize, orderDownloads, PLATFORM_NAMES } from "@/lib/release-assets";
import { RELEASES_URL, REPO_URL } from "@/lib/site";

export const metadata: Metadata = {
  title: "Download Commy",
  description: "Download Commy for Windows, macOS and Linux. Free and open source, no account required for P2P chat.",
};

export default async function DownloadPage() {
  const release = await getLatestRelease();
  const platform = detectPlatform(headers().get("user-agent"));
  const downloads = release ? orderDownloads(release.downloads, platform) : [];
  const recommended = downloads.find((d) => d.platform === platform && d.primary) ?? null;

  return (
    <main className="page">
      <h1>Download Commy</h1>

      {release && downloads.length > 0 ? (
        <>
          <p className="muted">
            Latest release{" "}
            <a href={release.url} target="_blank" rel="noreferrer">
              {release.name}
            </a>
            , published {new Date(release.publishedAt).toLocaleDateString("en-US", { dateStyle: "long" })}.
          </p>

          {recommended && (
            <div className="account-card download-hero">
              <p className="muted">Recommended for your computer</p>
              <a href={recommended.url} className="btn btn-primary btn-large">
                Download for {PLATFORM_NAMES[recommended.platform]}
              </a>
              <p className="muted small">
                {recommended.kind}
                {formatSize(recommended.size) && `, ${formatSize(recommended.size)}`}
              </p>
            </div>
          )}

          <h2>All downloads</h2>
          <ul className="download-list">
            {downloads.map((d) => (
              <li key={d.url}>
                <div>
                  <p>{d.label}</p>
                  <p className="muted small">
                    {d.kind}
                    {formatSize(d.size) && `, ${formatSize(d.size)}`}
                  </p>
                </div>
                <a href={d.url} className={d.primary ? "btn btn-primary" : "btn"}>
                  Download
                </a>
              </li>
            ))}
          </ul>

          <p className="muted small">
            {release.checksums && (
              <>
                Verify your download with the{" "}
                <a href={release.checksums}>SHA-256 checksums</a>.{" "}
              </>
            )}
            Older versions and release notes are on{" "}
            <a href={RELEASES_URL} target="_blank" rel="noreferrer">
              GitHub Releases
            </a>
            .
          </p>
        </>
      ) : (
        <div className="account-card">
          <p>
            The first public release is on its way. Watch{" "}
            <a href={RELEASES_URL} target="_blank" rel="noreferrer">
              GitHub Releases
            </a>{" "}
            to be notified, or build Commy from source below.
          </p>
        </div>
      )}

      <h2>After installing</h2>
      <p>
        Pick a username and you are ready: chat, voice and files work peer to peer with no account. Sign in from
        Settings to link your devices, and see <Link href="/docs/getting-started">Getting started</Link> for the
        rest.
      </p>

      <h2>Build from source</h2>
      <p>Commy is written in Rust with a Tauri shell:</p>
      <pre>
        <code>{`git clone ${REPO_URL}
cd commy
cargo run -p commy-desktop`}</code>
      </pre>
    </main>
  );
}
