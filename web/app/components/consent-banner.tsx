"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { onConsentSettingsOpen, openConsentSettings, readConsent, saveConsent, type Consent } from "@/lib/consent";

// Shown until the visitor decides. Reject is one click, same as Accept.
export default function ConsentBanner() {
  const [open, setOpen] = useState(false);
  const [current, setCurrent] = useState<Consent | null>(null);

  useEffect(() => {
    const value = readConsent();
    setCurrent(value);
    setOpen(value === null);
    return onConsentSettingsOpen(() => {
      setCurrent(readConsent());
      setOpen(true);
    });
  }, []);

  if (!open) return null;

  const choose = (value: Consent) => {
    saveConsent(value);
    setCurrent(value);
    setOpen(false);
  };

  return (
    <div className="consent" role="dialog" aria-modal="false" aria-labelledby="consent-title">
      <div className="consent-body">
        <h2 id="consent-title">Cookies on commy.chat</h2>
        <p>
          We do not use analytics, ads or tracking. The only optional extra is Cloudflare&apos;s security check on
          the sign-in and sign-up forms, which may store data on your device. It stays off unless you accept.
          {current && <> Right now it is <strong>{current === "accepted" ? "allowed" : "off"}</strong>.</>}{" "}
          <Link href="/privacy#cookies">Details</Link>
        </p>
      </div>
      <div className="consent-actions">
        <button type="button" className="btn btn-ghost" onClick={() => choose("rejected")}>
          Reject
        </button>
        <button type="button" className="btn btn-primary" onClick={() => choose("accepted")}>
          Accept
        </button>
      </div>
    </div>
  );
}

export function ConsentSettingsLink() {
  return (
    <button
      type="button"
      className="link-button footer-link"
      onClick={openConsentSettings}
    >
      Cookie settings
    </button>
  );
}
