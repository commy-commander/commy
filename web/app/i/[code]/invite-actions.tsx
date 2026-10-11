"use client";

import Link from "next/link";
import { useState } from "react";

// commy://i/<host>/<code> tells the app which server to resolve the code on.
export function deepLink(host: string, code: string) {
  return `commy://i/${encodeURIComponent(host)}/${encodeURIComponent(code)}`;
}

export default function InviteActions({ host, code, url }: { host: string; code: string; url: string }) {
  const [opened, setOpened] = useState(false);
  const [copied, setCopied] = useState(false);

  return (
    <>
      <div className="hero-actions invite-actions">
        <a href={deepLink(host, code)} className="btn btn-primary" onClick={() => setOpened(true)}>
          Open in Commy
        </a>
        <button
          type="button"
          className="btn"
          onClick={async () => {
            await navigator.clipboard.writeText(url).catch(() => undefined);
            setCopied(true);
          }}
        >
          {copied ? "Copied" : "Copy invite link"}
        </button>
      </div>
      {opened && (
        <p className="muted small">
          Nothing happened? <Link href="/download">Download Commy</Link>, then paste this link into Add a Server, Join
          Server.
        </p>
      )}
    </>
  );
}
