"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

const LINKS = [
  { href: "/docs", label: "Docs" },
  { href: "/account", label: "Account" },
];

// Wordmark, three links and one action. On a phone the links fold into a Menu sheet.
export default function NavLinks({ repoUrl }: { repoUrl: string }) {
  const pathname = usePathname();
  const current = (href: string) => (pathname === href || pathname.startsWith(`${href}/`) ? "page" : undefined);

  return (
    <>
      <nav className="nav-links" aria-label="Main">
        {LINKS.map((l) => (
          <Link key={l.href} href={l.href} aria-current={current(l.href)}>
            {l.label}
          </Link>
        ))}
        <a href={repoUrl} target="_blank" rel="noreferrer">
          Source
        </a>
        <Link href="/download" className="btn btn-primary" aria-current={current("/download")}>
          Download
        </Link>
      </nav>
      <details className="menu">
        <summary>Menu</summary>
        <nav aria-label="Main">
          <Link href="/download" aria-current={current("/download")}>
            Download
          </Link>
          {LINKS.map((l) => (
            <Link key={l.href} href={l.href} aria-current={current(l.href)}>
              {l.label}
            </Link>
          ))}
          <a href={repoUrl} target="_blank" rel="noreferrer">
            Source
          </a>
        </nav>
      </details>
    </>
  );
}
