import { useEffect, useState } from "react";
import { keyFromHash, openBackground, openInvite } from "../lib/sealed-invite.js";
import { useSite } from "../site.jsx";

function deepLink(host, code, key) {
  const link = `commy://i/${encodeURIComponent(host)}/${encodeURIComponent(code)}`;
  return key ? `${link}#${key}` : link;
}

export function webAppLink(webApp, fullUrl) {
  return webApp ? `${webApp}#join=${encodeURIComponent(fullUrl)}` : null;
}

function formatExpiry(at) {
  const soon = at.getTime() - Date.now() < 36 * 3600 * 1000;
  return soon
    ? at.toLocaleString("en-US", { month: "long", day: "numeric", hour: "numeric", minute: "2-digit" })
    : at.toLocaleDateString("en-US", { month: "long", day: "numeric" });
}

async function loadBackground(code, key) {
  const res = await fetch(`/api/auth/commy/invites/background?code=${encodeURIComponent(code)}`, {
    credentials: "omit",
  }).catch(() => null);
  if (!res?.ok) return null;
  const body = await res.json().catch(() => null);
  return typeof body?.background === "string" ? openBackground(body.background, key) : null;
}

export default function InviteActions({ host, code, url, sealed, plainName, expiresAt, hasBackground }) {
  const { links, webApp } = useSite();
  const [opened, setOpened] = useState(false);
  const [copied, setCopied] = useState(false);
  const [key, setKey] = useState(null);
  const [name, setName] = useState(plainName);
  const [broken, setBroken] = useState(false);
  const [expired, setExpired] = useState(false);
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  const [expiry, setExpiry] = useState(expiresAt ? new Date(expiresAt) : null);
  const [background, setBackground] = useState(null);

  useEffect(() => {
    if (!hasBackground || !key) return;
    let objectUrl = null;
    let live = true;
    loadBackground(code, key).then((blob) => {
      if (!blob || !live) return;
      objectUrl = URL.createObjectURL(blob);
      setBackground(objectUrl);
    });
    return () => {
      live = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [hasBackground, code, key]);

  useEffect(() => {
    if (!sealed) return;
    const found = keyFromHash(window.location.hash);
    setKey(found);
    if (!found) {
      setBroken(true);
      return;
    }
    openInvite(sealed, found).then((opened) => {
      if (!opened?.name) {
        setBroken(true);
        return;
      }
      setName(opened.name);
      if (typeof opened.expires === "number") {
        const at = new Date(opened.expires * 1000);
        if (at.getTime() <= Date.now()) setExpired(true);
        setExpiry((server) => (server && server < at ? server : at));
      }
    });
  }, [sealed]);

  if (expired) {
    return (
      <>
        <h1 className="invite-name">This invite has expired</h1>
        <p className="muted small">Ask whoever sent it for a new link.</p>
      </>
    );
  }

  if (broken) {
    return (
      <>
        <h1 className="invite-name">This link is incomplete</h1>
        <p className="muted small">
          Part of the invite link is missing, so it cannot be opened. Ask whoever sent it to copy the whole link again.
        </p>
      </>
    );
  }

  const fullUrl = key ? `${url}#${key}` : url;

  return (
    <>
      {hasBackground && (
        <div className="invite-banner" aria-hidden="true">
          {background && <img src={background} alt="" />}
        </div>
      )}
      <p className="muted">You have been invited to join</p>
      <h1 className="invite-name">{name ?? "a Commy server"}</h1>
      <p className="muted small">
        Invite hosted by {host} · {expiry ? (mounted ? `expires ${formatExpiry(expiry)}` : "expires") : "never expires"}
      </p>
      <div className="hero-actions invite-actions">
        <a href={deepLink(host, code, key)} className="btn btn-primary" onClick={() => setOpened(true)}>
          Open in Commy
        </a>
        {webApp && (
          <a href={webAppLink(webApp, fullUrl)} className="btn">
            Join in the browser
          </a>
        )}
        <button
          type="button"
          className="btn"
          onClick={async () => {
            await navigator.clipboard.writeText(fullUrl).catch(() => undefined);
            setCopied(true);
          }}
        >
          {copied ? "Copied" : "Copy invite link"}
        </button>
      </div>
      <p className="muted small invite-hint">
        Signed in on another Commy server? Copy the link and paste it into Join Server in the app or web app you use
        there.
      </p>
      {opened && (
        <p className="muted small invite-hint">
          Nothing happened? <a href={links.download}>Download Commy</a>, then paste this link into Add a Server, Join
          Server.
        </p>
      )}
    </>
  );
}
