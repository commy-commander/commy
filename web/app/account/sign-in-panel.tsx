"use client";

import { useRef, useState } from "react";
import { authPost } from "@/lib/auth-client";
import type { Providers } from "@/lib/auth";
import Turnstile, { type TurnstileHandle } from "./turnstile";

type Mode = "signin" | "signup";

export default function SignInPanel({
  providers,
  callbackURL,
}: {
  providers: Providers;
  callbackURL: string;
}) {
  const [mode, setMode] = useState<Mode>("signin");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const bot = useRef<TurnstileHandle>(null);
  const captcha = () => bot.current?.headers() ?? {};

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  const google = () =>
    run(async () => {
      const res = await authPost<{ url?: string }>("/sign-in/social", { provider: "google", callbackURL });
      if (res.url) window.location.href = res.url;
    });

  const submitPassword = (form: FormData) =>
    run(async () => {
      const email = String(form.get("email") ?? "");
      const password = String(form.get("password") ?? "");
      if (mode === "signup") {
        const res = await authPost<{ token: string | null }>("/sign-up/email", {
          name: String(form.get("name") ?? "") || email.split("@")[0],
          email,
          password,
          callbackURL,
        }, captcha());
        if (!res.token) {
          setNotice("Check your inbox to verify your email, then sign in.");
          setMode("signin");
          return;
        }
      } else {
        await authPost("/sign-in/email", { email, password, callbackURL }, captcha());
      }
      window.location.href = callbackURL;
    });

  const magicLink = (form: FormData) =>
    run(async () => {
      await authPost("/sign-in/magic-link", { email: String(form.get("email") ?? ""), callbackURL }, captcha());
      setNotice("Check your inbox for a sign-in link.");
    });

  return (
    <div className="account-card">
      {providers.google && (
        <>
          <button type="button" className="btn btn-primary btn-block" onClick={google} disabled={busy}>
            Continue with Google
          </button>
          <div className="divider">or</div>
        </>
      )}

      <form className="signin-form" action={submitPassword}>
        {mode === "signup" && <input name="name" placeholder="Display name" autoComplete="nickname" maxLength={64} />}
        <input type="email" name="email" placeholder="you@example.com" autoComplete="email" required />
        <input
          type="password"
          name="password"
          placeholder="Password"
          autoComplete={mode === "signup" ? "new-password" : "current-password"}
          minLength={8}
          required
        />
        <button type="submit" className="btn" disabled={busy}>
          {mode === "signup" ? "Create account" : "Sign in"}
        </button>
      </form>
      <p className="muted small">
        {mode === "signin" ? "New here? " : "Already have an account? "}
        <button type="button" className="link-button" onClick={() => setMode(mode === "signin" ? "signup" : "signin")}>
          {mode === "signin" ? "Create an account" : "Sign in"}
        </button>
      </p>

      {providers.magicLink && (
        <>
          <div className="divider">or</div>
          <form className="signin-form" action={magicLink}>
            <input type="email" name="email" placeholder="you@example.com" autoComplete="email" required />
            <button type="submit" className="btn" disabled={busy}>
              Email me a magic link
            </button>
          </form>
        </>
      )}

      <Turnstile ref={bot} siteKey={providers.turnstileSiteKey} />

      {error && <p className="form-error" role="alert">{error}</p>}
      {notice && <p className="form-notice" role="status">{notice}</p>}
    </div>
  );
}
