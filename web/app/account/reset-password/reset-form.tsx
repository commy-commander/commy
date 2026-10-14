"use client";

import { useState } from "react";
import { authPost } from "@/lib/auth-client";

export default function ResetPasswordForm({ token }: { token: string }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);

  async function submit(form: FormData) {
    const password = String(form.get("password") ?? "");
    if (password !== String(form.get("confirm") ?? "")) {
      setError("The two passwords do not match.");
      return;
    }
    setBusy(true);
    setError("");
    try {
      await authPost("/reset-password", { token, newPassword: password });
      setDone(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  if (done) {
    return (
      <div className="account-card">
        <p className="form-notice" role="status">
          Password changed. Every other device has been signed out.
        </p>
        <a className="btn btn-primary" href="/account">
          Sign in
        </a>
      </div>
    );
  }

  return (
    <div className="account-card">
      <form className="signin-form" action={submit}>
        <input type="password" name="password" placeholder="New password" autoComplete="new-password" minLength={8} required />
        <input type="password" name="confirm" placeholder="Repeat new password" autoComplete="new-password" minLength={8} required />
        <button type="submit" className="btn" disabled={busy}>
          Save new password
        </button>
      </form>
      {error && <p className="form-error" role="alert">{error}</p>}
    </div>
  );
}
