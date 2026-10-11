import type { Metadata } from "next";
import Link from "next/link";
import { LICENSE_NAME, RELEASES_URL, REPO_URL } from "@/lib/site";
import "./globals.css";

export const metadata: Metadata = {
  metadataBase: new URL(process.env.PUBLIC_URL || "https://commy.chat"),
  title: "Commy - P2P voice and chat, free forever",
  description:
    "Commy is free, open source, peer-to-peer voice and text chat for communities, written in Rust. Voice, file sharing, soundboards, and screen sharing with no paywall.",
  openGraph: { siteName: "Commy", images: ["/logo.png"] },
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>
        <header className="site-header">
          <Link href="/" className="brand">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src="/logo.png" alt="" width={28} height={28} />
            Commy
          </Link>
          <nav className="site-nav">
            <Link href="/docs">Docs</Link>
            <Link href="/download">Download</Link>
            <a href={REPO_URL} target="_blank" rel="noreferrer" className="nav-github">
              GitHub
            </a>
            <Link href="/account" className="nav-account">
              Account
            </Link>
          </nav>
        </header>
        {children}
        <footer className="site-footer">
          <div className="footer-columns">
            <div>
              <h4>Commy</h4>
              <Link href="/download">Download</Link>
              <Link href="/docs/getting-started">Getting started</Link>
              <Link href="/docs/invites">Invite links</Link>
            </div>
            <div>
              <h4>Open source</h4>
              <a href={REPO_URL} target="_blank" rel="noreferrer">
                Source code
              </a>
              <a href={RELEASES_URL} target="_blank" rel="noreferrer">
                Releases
              </a>
              <a href={`${REPO_URL}/issues`} target="_blank" rel="noreferrer">
                Report a bug
              </a>
            </div>
            <div>
              <h4>Trust</h4>
              <Link href="/docs/privacy">Privacy</Link>
              <Link href="/docs/self-hosting">Host your own</Link>
              <a href={`${REPO_URL}/blob/main/LICENSE`} target="_blank" rel="noreferrer">
                License ({LICENSE_NAME})
              </a>
            </div>
          </div>
          <p>Commy is free and open source. No paywalls. No servers to rent.</p>
        </footer>
      </body>
    </html>
  );
}
