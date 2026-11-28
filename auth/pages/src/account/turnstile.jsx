import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { onConsentChange, readConsent } from "../lib/consent.js";

const SCRIPT_SRC = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";

function loadScript() {
  if (window.turnstile) return Promise.resolve();
  return new Promise((resolve, reject) => {
    let script = document.querySelector(`script[src="${SCRIPT_SRC}"]`);
    if (!script) {
      script = document.createElement("script");
      script.src = SCRIPT_SRC;
      script.async = true;
      document.head.appendChild(script);
    }
    script.addEventListener("load", () => resolve());
    script.addEventListener("error", () => reject(new Error("Could not load the bot check")));
  });
}

const Turnstile = forwardRef(function Turnstile({ siteKey }, ref) {
  const el = useRef(null);
  const widget = useRef(null);
  const [consent, setConsent] = useState(undefined);
  const [allowedHere, setAllowedHere] = useState(false);
  const allowed = consent === "accepted" || allowedHere;

  useEffect(() => {
    setConsent(readConsent());
    return onConsentChange(setConsent);
  }, []);

  useEffect(() => {
    if (!siteKey || !allowed || !el.current) return;
    let cancelled = false;
    loadScript()
      .then(() => {
        if (!cancelled && el.current && window.turnstile && widget.current === null) {
          widget.current = window.turnstile.render(el.current, { sitekey: siteKey, theme: "dark", size: "flexible" });
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [siteKey, allowed]);

  useImperativeHandle(ref, () => ({
    unfinished: () => {
      if (!siteKey) return null;
      if (consent === null) return "Choose Accept or Reject in the cookie banner first.";
      if (!allowed) return "Allow the security check above the button first.";
      if (widget.current === null || !window.turnstile?.getResponse(widget.current)) {
        return "Finish the security check above the button first.";
      }
      return null;
    },
    headers: () => {
      if (!siteKey || widget.current === null || !window.turnstile) return {};
      const token = window.turnstile.getResponse(widget.current) ?? "";
      window.turnstile.reset(widget.current);
      return token ? { "x-captcha-response": token } : {};
    },
  }));

  if (!siteKey) return null;
  if (consent === undefined) return <div className="turnstile" />;
  if (consent === null) {
    return (
      <p className="turnstile-consent muted small" role="status">
        Before you continue, choose Accept or Reject in the cookie banner. Accept turns on Cloudflare&apos;s security
        check for this form. If you reject, you can still allow it for this form only.
      </p>
    );
  }
  if (!allowed) {
    return (
      <div className="turnstile-consent">
        <p className="muted small">
          Signing in needs Cloudflare&apos;s security check, which stays off until you allow it.
        </p>
        <button type="button" className="btn" onClick={() => setAllowedHere(true)}>
          Allow the security check for this form
        </button>
      </div>
    );
  }
  return <div ref={el} className="turnstile" />;
});

export default Turnstile;
