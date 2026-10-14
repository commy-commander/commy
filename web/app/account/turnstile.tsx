"use client";

import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { onConsentChange, readConsent } from "@/lib/consent";

type TurnstileApi = {
  render: (el: HTMLElement, opts: { sitekey: string; theme?: string }) => string;
  getResponse: (id: string) => string | undefined;
  reset: (id: string) => void;
};

declare global {
  interface Window {
    turnstile?: TurnstileApi;
  }
}

const SCRIPT_SRC = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";

function loadScript(): Promise<void> {
  if (window.turnstile) return Promise.resolve();
  return new Promise((resolve, reject) => {
    let script = document.querySelector<HTMLScriptElement>(`script[src="${SCRIPT_SRC}"]`);
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

export type TurnstileHandle = {
  // Header for protected auth calls; tokens are single use, so this resets the widget
  headers: () => Record<string, string>;
};

// Cloudflare Turnstile bot check, rendered only when the server has a site key.
// The script is third party and may store data, so it loads only after the
// visitor accepts cookies or allows it for this form.
const Turnstile = forwardRef<TurnstileHandle, { siteKey: string | null }>(function Turnstile({ siteKey }, ref) {
  const el = useRef<HTMLDivElement>(null);
  const widget = useRef<string | null>(null);
  const [allowed, setAllowed] = useState(false);

  useEffect(() => {
    setAllowed(readConsent() === "accepted");
    return onConsentChange((value) => setAllowed(value === "accepted"));
  }, []);

  useEffect(() => {
    if (!siteKey || !allowed || !el.current) return;
    let cancelled = false;
    loadScript()
      .then(() => {
        if (!cancelled && el.current && window.turnstile && widget.current === null) {
          widget.current = window.turnstile.render(el.current, { sitekey: siteKey, theme: "dark" });
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [siteKey, allowed]);

  useImperativeHandle(ref, () => ({
    headers: (): Record<string, string> => {
      if (!siteKey || widget.current === null || !window.turnstile) return {};
      const token = window.turnstile.getResponse(widget.current) ?? "";
      window.turnstile.reset(widget.current);
      return token ? { "x-captcha-response": token } : {};
    },
  }));

  if (!siteKey) return null;
  if (!allowed) {
    return (
      <div className="turnstile-consent">
        <p className="muted small">
          Signing in needs Cloudflare&apos;s security check, which stays off until you allow it.
        </p>
        <button type="button" className="btn" onClick={() => setAllowed(true)}>
          Allow the security check for this form
        </button>
      </div>
    );
  }
  return <div ref={el} className="turnstile" />;
});

export default Turnstile;
