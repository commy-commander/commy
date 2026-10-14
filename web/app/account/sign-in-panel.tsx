"use client";

import { useRef, useState } from "react";
import { AuthError, authPost } from "@/lib/auth-client";
import type { Providers } from "@/lib/auth";
import Turnstile, { type TurnstileHandle } from "./turnstile";

type Mode = "signin" | "signup" | "forgot";
type Social = "google" | "twitch";

const SOCIAL_LABELS: Record<Social, string> = {
  google: "Continue with Google",
  twitch: "Continue with Twitch",
};

export default function SignInPanel({ providers, callbackURL }: { providers: Providers; callbackURL: string }) {
  const [mode, setMode] = useState<Mode>("signin");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  // Set when sign-in was refused because the address is not verified yet
  const [unverified, setUnverified] = useState("");
  const bot = useRef<TurnstileHandle>(null);
  const captcha = () => bot.current?.headers() ?? {};
  const socials = (["google", "twitch"] as const).filter((p) => providers[p]);

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

  const social = (provider: Social) =>
    run(async () => {
      const res = await authPost<{ url?: string }>("/sign-in/social", {
        provider,
        callbackURL,
        errorCallbackURL: "/account",
      });
      if (res.url) window.location.href = res.url;
    });

  const submitPassword = (form: FormData) =>
    run(async () => {
      const email = String(form.get("email") ?? "");
      const password = String(form.get("password") ?? "");
      setUnverified("");
      if (mode === "signup") {
        const res = await authPost<{ token: string | null }>(
          "/sign-up/email",
          {
            name: String(form.get("name") ?? "") || email.split("@")[0],
            email,
            password,
            callbackURL,
          },
          captcha(),
        );
        if (!res.token) {
          setMode("signin");
          setUnverified(email);
          setNotice(`We sent a verification link to ${email}. Open it to finish creating your account.`);
          return;
        }
      } else {
        try {
          await authPost("/sign-in/email", { email, password, callbackURL }, captcha());
        } catch (err) {
          if (err instanceof AuthError && err.code === "EMAIL_NOT_VERIFIED") {
            setUnverified(email);
            setNotice(`Verify your email first. We sent a new link to ${email}.`);
            return;
          }
          throw err;
        }
      }
      window.location.href = callbackURL;
    });

  const resend = () =>
    run(async () => {
      await authPost("/send-verification-email", { email: unverified, callbackURL }, captcha());
      setNotice(`Sent another verification link to ${unverified}. Check your spam folder if it does not arrive.`);
    });

  const forgot = (form: FormData) =>
    run(async () => {
      await authPost(
        "/request-password-reset",
        {
          email: String(form.get("email") ?? ""),
          redirectTo: `${window.location.origin}/account/reset-password`,
        },
        captcha(),
      );
      setNotice("If an account uses that address, a reset link is on its way. It expires in 1 hour.");
    });

  const magicLink = (form: FormData) =>
    run(async () => {
      await authPost("/sign-in/magic-link", { email: String(form.get("email") ?? ""), callbackURL }, captcha());
      setNotice("Check your inbox for a sign-in link.");
    });

  if (mode === "forgot") {
    return (
      <div className="account-card">
        <h2>Reset your password</h2>
        <form className="signin-form" action={forgot}>
          <label>
            <span>Email</span>
            <input type="email" name="email" placeholder="you@example.com" autoComplete="email" required />
          </label>
          <button type="submit" className="btn" disabled={busy}>
            Email me a reset link
          </button>
        </form>
        <p className="muted small">
          <button type="button" className="link-button" onClick={() => setMode("signin")}>
            Back to sign in
          </button>
        </p>
        <Turnstile ref={bot} siteKey={providers.turnstileSiteKey} />
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        {notice && (
          <p className="form-notice" role="status">
            {notice}
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="account-card">
      {socials.length > 0 && (
        <>
          <div className="social-buttons">
            {socials.map((p) => (
              <button
                key={p}
                type="button"
                className={`btn btn-block btn-social btn-${p}`}
                onClick={() => social(p)}
                disabled={busy}
              >
                {SOCIAL_LABELS[p]}
              </button>
            ))}
          </div>
          <div className="divider">or</div>
        </>
      )}

      <form className="signin-form" action={submitPassword}>
        {mode === "signup" && (
          <label>
            <span>Display name</span>
            <input name="name" placeholder="Display name" autoComplete="nickname" maxLength={64} />
          </label>
        )}
        <label>
          <span>Email</span>
          <input type="email" name="email" placeholder="you@example.com" autoComplete="email" required />
        </label>
        <label>
          <span>Password</span>
          <input
            type="password"
            name="password"
            placeholder="Password"
            autoComplete={mode === "signup" ? "new-password" : "current-password"}
            minLength={8}
            required
          />
        </label>
        <button type="submit" className="btn" disabled={busy}>
          {mode === "signup" ? "Create account" : "Sign in"}
        </button>
      </form>
      <p className="muted small">
        {mode === "signin" ? "New here? " : "Already have an account? "}
        <button type="button" className="link-button" onClick={() => setMode(mode === "signin" ? "signup" : "signin")}>
          {mode === "signin" ? "Create an account" : "Sign in"}
        </button>
        {mode === "signin" && providers.passwordReset && (
          <>
            {" · "}
            <button type="button" className="link-button" onClick={() => setMode("forgot")}>
              Forgot password?
            </button>
          </>
        )}
      </p>

      {providers.magicLink && (
        <>
          <div className="divider">or</div>
          <form className="signin-form" action={magicLink}>
            <label>
              <span>Email</span>
              <input type="email" name="email" placeholder="you@example.com" autoComplete="email" required />
            </label>
            <button type="submit" className="btn" disabled={busy}>
              Email me a magic link
            </button>
          </form>
        </>
      )}

      <Turnstile ref={bot} siteKey={providers.turnstileSiteKey} />

      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <p className="form-notice" role="status">
          {notice}
        </p>
      )}
      {unverified && (
        <p className="muted small">
          No email?{" "}
          <button type="button" className="link-button" onClick={resend} disabled={busy}>
            Resend verification link
          </button>
        </p>
      )}
    </div>
  );
}
