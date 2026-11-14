import { useState } from "react";
import { isStaleSession, SIGN_IN_AGAIN, signOutAndReturn } from "../lib/auth-client.js";
import { errorMessage } from "../lib/use-action.js";

export function useSecureAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [stale, setStale] = useState(false);
  const [notice, setNotice] = useState("");

  async function run(action) {
    setBusy(true);
    setError("");
    setStale(false);
    setNotice("");
    try {
      await action();
    } catch (err) {
      const needsSignIn = isStaleSession(err);
      setStale(needsSignIn);
      setError(needsSignIn ? SIGN_IN_AGAIN : errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return { busy, error, setError, stale, notice, setNotice, run };
}

export function Feedback({ error, stale, notice }) {
  return (
    <>
      {error && (
        <p className="form-error" role="alert">
          {error}{" "}
          {stale && (
            <button type="button" className="link-button" onClick={() => signOutAndReturn("/account")}>
              Sign in again
            </button>
          )}
        </p>
      )}
      {notice && (
        <p className="form-notice" role="status">
          {notice}
        </p>
      )}
    </>
  );
}

export function PasswordField({ label = "Current password", name = "password" }) {
  return (
    <label>
      <span>{label}</span>
      <input type="password" name={name} autoComplete="current-password" required />
    </label>
  );
}

export function NewPasswordFields() {
  return (
    <>
      <label>
        <span>New password</span>
        <input type="password" name="newPassword" autoComplete="new-password" minLength={8} maxLength={128} required />
        <small className="field-hint">At least 8 characters.</small>
      </label>
      <label>
        <span>Repeat new password</span>
        <input type="password" name="confirm" autoComplete="new-password" minLength={8} maxLength={128} required />
      </label>
    </>
  );
}

export function readNewPassword(form) {
  const password = String(form.get("newPassword") ?? "");
  if (password !== String(form.get("confirm") ?? "")) throw new Error("The two passwords do not match.");
  return password;
}
