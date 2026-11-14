import { useEffect, useState } from "react";
import { onConsentSettingsOpen, readConsent, saveConsent } from "./lib/consent.js";

export default function ConsentBanner({ host, privacy }) {
  const [open, setOpen] = useState(false);
  const [current, setCurrent] = useState(null);

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

  const choose = (value) => {
    saveConsent(value);
    setCurrent(value);
    setOpen(false);
  };

  return (
    <div className="consent" role="dialog" aria-modal="false" aria-labelledby="consent-title">
      <div className="consent-body">
        <h2 id="consent-title">Cookies on {host}</h2>
        <p>
          We do not use analytics, ads or tracking. The only optional extra is Cloudflare&apos;s security check on
          the sign-in and sign-up forms, which may store data on your device. It stays off unless you accept, or allow it on that form.
          {current && <> Right now it is <strong>{current === "accepted" ? "allowed" : "off"}</strong>.</>}
          {privacy && (
            <>
              {" "}
              <a href={privacy}>Details</a>
            </>
          )}
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
