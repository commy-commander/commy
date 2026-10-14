import type { Metadata } from "next";
import Link from "next/link";
import { headers } from "next/headers";
import { getLatestRelease } from "@/lib/releases";
import { detectPlatform, formatSize, PLATFORM_NAMES, type Download, type Platform } from "@/lib/release-assets";
import { RELEASES_URL } from "@/lib/site";
import Mascot from "../components/mascot";

export const metadata: Metadata = {
  title: "Download Commy",
  description: "Download Commy for Windows, macOS and Linux. Free and open source chat, voice and streaming.",
};

const PLATFORMS: { id: Platform; note: string }[] = [
  { id: "windows", note: "Windows 10 and 11, 64-bit" },
  { id: "macos", note: "Apple silicon and Intel" },
  { id: "linux", note: "64-bit, most distributions" },
];

export default async function DownloadPage() {
  const release = await getLatestRelease();
  const platform = detectPlatform(headers().get("user-agent"));
  const downloads = release?.downloads ?? [];
  const recommended = downloads.find((d) => d.platform === platform && d.primary) ?? null;
  const published = release ? new Date(release.publishedAt).toLocaleDateString("en-US", { dateStyle: "long" }) : null;

  if (!release || downloads.length === 0) {
    return (
      <main className="dl">
        <section className="dl-hero">
          <div>
            <h1>The first release is coming</h1>
            <p className="lead">
              Commy for Windows, macOS and Linux will be downloadable here the moment the first version is published.
            </p>
            <div className="hero-actions">
              <a href={RELEASES_URL} className="btn btn-primary btn-large" target="_blank" rel="noreferrer">
                Watch releases on GitHub
              </a>
            </div>
            <p className="dl-meta">Installers, checksums and release notes will all be listed on this page.</p>
          </div>
          <Mascot className="dl-mark" />
        </section>
        <Platforms downloads={[]} hashes={{}} current={platform} />
      </main>
    );
  }

  return (
    <main className="dl">
      <section className="dl-hero">
        <div>
          <h1>{recommended ? `Commy for ${PLATFORM_NAMES[recommended.platform]}` : "Download Commy"}</h1>
          <p className="lead">Free and open source chat, voice and streaming for your community.</p>
          <div className="hero-actions">
            {recommended ? (
              <a href={recommended.url} className="btn btn-primary btn-large">
                Download {release.name}
              </a>
            ) : (
              <a href="#platforms" className="btn btn-primary btn-large">
                Choose your platform
              </a>
            )}
          </div>
          <p className="dl-meta">
            {recommended && (
              <>
                {recommended.label}, {recommended.kind}
                {formatSize(recommended.size) && `, ${formatSize(recommended.size)}`}.{" "}
              </>
            )}
            Version{" "}
            <a href={release.url} target="_blank" rel="noreferrer">
              {release.name}
            </a>
            , published {published}.
          </p>
        </div>
        <Mascot className="dl-mark" />
      </section>

      <Platforms downloads={downloads} hashes={release.hashes} current={platform} />

      <section className="dl-notes">
        <div>
          <h2>Check your download</h2>
          <p>Every file has a SHA-256 checksum listed above. Compare it with the file you downloaded:</p>
          <pre>
            <code>{`# Windows (PowerShell)
Get-FileHash .\\Commy_*_x64-setup.exe
# macOS and Linux
shasum -a 256 Commy_*`}</code>
          </pre>
          {release.checksums && (
            <p>
              Or grab <a href={release.checksums}>SHA256SUMS.txt</a>.
            </p>
          )}
        </div>
        <div>
          <h2>After installing</h2>
          <p>
            Open Commy, sign in with a free account, then create a server or open an invite link from a friend. The{" "}
            <Link href="/docs/getting-started">getting started guide</Link> walks through the rest.
          </p>
          <p>
            Release notes and older versions are on{" "}
            <a href={RELEASES_URL} target="_blank" rel="noreferrer">
              GitHub Releases
            </a>
            .
          </p>
        </div>
      </section>
    </main>
  );
}

function Platforms({
  downloads,
  hashes,
  current,
}: {
  downloads: Download[];
  hashes: Record<string, string>;
  current: Platform | null;
}) {
  const order = [...PLATFORMS].sort((a, b) => Number(b.id === current) - Number(a.id === current));
  return (
    <section id="platforms" className="dl-platforms" aria-label="All downloads">
      {order.map((p) => {
        const files = downloads.filter((d) => d.platform === p.id).sort((a, b) => Number(b.primary) - Number(a.primary));
        return (
          <div key={p.id} className={`dl-platform ${p.id === current ? "current" : ""}`}>
            <div>
              <h2>{PLATFORM_NAMES[p.id]}</h2>
              <p className="muted small">{p.note}</p>
              {p.id === current && <span className="dl-tag">Your computer</span>}
            </div>
            <div className="dl-files">
              {files.length === 0 ? (
                <p className="muted">Coming with the first release.</p>
              ) : (
                files.map((d) => (
                  <div className="dl-file" key={d.url}>
                    <div className="dl-file-row">
                      <div>
                        <p className="name">{d.label}</p>
                        <p className="muted small">
                          {d.kind}
                          {formatSize(d.size) && `, ${formatSize(d.size)}`}
                        </p>
                      </div>
                      <a href={d.url} className={d.primary ? "btn btn-primary" : "btn btn-ghost"} aria-label={`Download ${d.file}`}>
                        Download
                      </a>
                    </div>
                    {hashes[d.file] && (
                      <code className="hash" title={`SHA-256 of ${d.file}`}>
                        SHA-256 {hashes[d.file]}
                      </code>
                    )}
                  </div>
                ))
              )}
            </div>
          </div>
        );
      })}
    </section>
  );
}
