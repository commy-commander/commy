import type { Metadata, Viewport } from "next";
import Link from "next/link";
import { LICENSE_NAME, REPO_URL } from "@/lib/site";
import ConsentBanner, { ConsentSettingsLink } from "./components/consent-banner";
import Mascot from "./components/mascot";
import NavLinks from "./components/nav-links";
import Reveal from "./components/reveal";
import "./globals.css";
import "./site.css";

export const metadata: Metadata = {
  metadataBase: new URL(process.env.PUBLIC_URL || "https://commy.chat"),
  title: "Commy. Voice and chat that goes straight to your friends",
  description:
    "Commy is free, open source, peer-to-peer voice and text chat for communities. Rich chat, clear voice, screen and game streaming, roles and invite links, with nothing in the middle.",
  openGraph: { siteName: "Commy", images: [{ url: "/og.png", width: 1200, height: 630 }] },
  twitter: { card: "summary_large_image", images: ["/og.png"] },
};

export const viewport: Viewport = { themeColor: "#f3f2ee" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  const year = new Date().getFullYear();
  return (
    <html lang="en">
      <head>
        <link rel="preload" href="/fonts/bricolage-latin.woff2" as="font" type="font/woff2" crossOrigin="anonymous" />
      </head>
      <body>
        {/* Motion only switches on once a script has run, so nothing is ever hidden without JavaScript. */}
        <script dangerouslySetInnerHTML={{ __html: "document.documentElement.classList.add('js')" }} />
        <a href="#main" className="skip-link">
          Skip to content
        </a>
        <header className="nav">
          <Link href="/" className="wordmark" aria-label="Commy home">
            <Mascot />
            Commy
          </Link>
          <NavLinks repoUrl={REPO_URL} />
        </header>
        <div id="main">{children}</div>
        <footer className="footer">
          <p>Commy is free and open source.</p>
          <nav aria-label="Footer">
            <Link href="/download">Download</Link>
            <Link href="/docs">Docs</Link>
            <a href={REPO_URL} target="_blank" rel="noreferrer">
              Source
            </a>
            <a href={`${REPO_URL}/blob/main/LICENSE`} target="_blank" rel="noreferrer">
              {LICENSE_NAME}
            </a>
            <ConsentSettingsLink />
          </nav>
          <p className="footer-right">
            {year} · <Link href="/docs/privacy">Privacy</Link>
          </p>
        </footer>
        <ConsentBanner />
        <Reveal />
      </body>
    </html>
  );
}
