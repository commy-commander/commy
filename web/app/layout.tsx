import type { Metadata, Viewport } from "next";
import localFont from "next/font/local";
import Link from "next/link";
import { LICENSE_NAME, RELEASES_URL, REPO_URL } from "@/lib/site";
import ConsentBanner, { ConsentSettingsLink } from "./components/consent-banner";
import Icon from "./components/icons";
import Reveal from "./components/reveal";
import "./globals.css";
import "./site.css";

// Self-hosted so no font request ever leaves the site.
const inter = localFont({
  src: [
    { path: "./fonts/inter-400.woff2", weight: "400" },
    { path: "./fonts/inter-500.woff2", weight: "500" },
    { path: "./fonts/inter-600.woff2", weight: "600" },
    { path: "./fonts/inter-700.woff2", weight: "700" },
    { path: "./fonts/inter-800.woff2", weight: "800" },
  ],
  variable: "--font-inter",
  display: "swap",
});

export const metadata: Metadata = {
  metadataBase: new URL(process.env.PUBLIC_URL || "https://commy.chat"),
  title: "Commy - free, open source voice and chat for your community",
  description:
    "Commy is free, open source, peer-to-peer voice and text chat for communities. Rich chat, clear voice, screen and game streaming, roles and invite links, with no paywall.",
  openGraph: { siteName: "Commy", images: [{ url: "/og.png", width: 1200, height: 630 }] },
  twitter: { card: "summary_large_image", images: ["/og.png"] },
};

export const viewport: Viewport = { themeColor: "#0b0d17" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={inter.variable}>
      <body>
        <a href="#main" className="skip-link">
          Skip to content
        </a>
        <header className="site-header">
          <Link href="/" className="brand">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src="/logo.png" alt="" width={32} height={32} />
            Commy
          </Link>
          <nav className="site-nav" aria-label="Main">
            <Link href="/#features">Features</Link>
            <Link href="/download">Download</Link>
            <Link href="/docs">Docs</Link>
            <a href={REPO_URL} target="_blank" rel="noreferrer" className="nav-github">
              <Icon name="github" size={18} /> GitHub
            </a>
            <Link href="/account" className="nav-account">
              Account
            </Link>
          </nav>
          <details className="mobile-nav">
            <summary aria-label="Menu">
              <Icon name="menu" />
            </summary>
            <nav aria-label="Mobile">
              <Link href="/#features">Features</Link>
              <Link href="/download">Download</Link>
              <Link href="/docs">Docs</Link>
              <a href={REPO_URL} target="_blank" rel="noreferrer">
                GitHub
              </a>
              <Link href="/account">Account</Link>
            </nav>
          </details>
        </header>
        <div id="main">{children}</div>
        <footer className="site-footer">
          <div className="footer-columns">
            <div className="footer-brand">
              <Link href="/" className="brand">
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img src="/logo.png" alt="" width={32} height={32} />
                Commy
              </Link>
              <p>Free and open source. No paywalls. No servers to rent.</p>
            </div>
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
              <ConsentSettingsLink />
              <Link href="/docs/self-hosting">Host your own</Link>
              <a href={`${REPO_URL}/blob/main/LICENSE`} target="_blank" rel="noreferrer">
                License ({LICENSE_NAME})
              </a>
            </div>
          </div>
        </footer>
        <ConsentBanner />
        <Reveal />
      </body>
    </html>
  );
}
