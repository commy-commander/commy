import { useState } from "react";
import { AuthError, authPost } from "../lib/auth-client.js";
import { useAction } from "../lib/use-action.js";
import { submitForm } from "../lib/forms.js";

const EXPIRED_CODES = new Set(["INVALID_TWO_FACTOR_COOKIE", "TOO_MANY_ATTEMPTS_REQUEST_NEW_CODE"]);

export default function TwoFactorForm({ next }) {
  const [backup, setBackup] = useState(false);
  const [expired, setExpired] = useState(false);
  const { busy, error, run } = useAction();

  const submit = (form) =>
    run(async () => {
      const code = String(form.get("code") ?? "").trim();
      try {
        await authPost(backup ? "/two-factor/verify-backup-code" : "/two-factor/verify-totp", {
          code: backup ? code : code.replace(/\s+/g, ""),
        });
      } catch (err) {
        if (err instanceof AuthError && err.code && EXPIRED_CODES.has(err.code)) {
          setExpired(true);
          return;
        }
        throw err instanceof AuthError && err.status === 401 ? new Error("That code did not work. Try the newest one.") : err;
      }
      window.location.href = next;
    });

  if (expired) {
    return (
      <div className="account-card">
        <p className="form-error" role="alert">
          This sign-in expired or had too many wrong codes.
        </p>
        <p className="muted">
          <a href={next}>Sign in again</a> to get a fresh one.
        </p>
      </div>
    );
  }

  return (
    <div className="account-card">
      <form className="signin-form" onSubmit={submitForm(submit)} key={backup ? "backup" : "totp"}>
        <label>
          <span>{backup ? "Backup code" : "Code from your authenticator app"}</span>
          {backup ? (
            <input className="code-input" name="code" autoComplete="off" spellCheck={false} maxLength={32} required />
          ) : (
            <input
              className="code-input"
              name="code"
              inputMode="numeric"
              autoComplete="one-time-code"
              pattern="[0-9 ]{6,7}"
              maxLength={7}
              autoFocus
              required
            />
          )}
        </label>
        <button type="submit" className="btn btn-primary btn-block" disabled={busy}>
          Continue
        </button>
      </form>
      <div className="signin-links">
        <button type="button" className="link-button" onClick={() => setBackup(!backup)}>
          {backup ? "Use your authenticator app" : "Lost your authenticator? Use a backup code"}
        </button>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
