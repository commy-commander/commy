import { useState } from "react";
import { authPost } from "../lib/auth-client.js";
import { useAction } from "../lib/use-action.js";
import { submitForm } from "../lib/forms.js";

export default function ResetPasswordForm({ token }) {
  const { busy, error, setError, run } = useAction();
  const [done, setDone] = useState(false);

  async function submit(form) {
    const password = String(form.get("password") ?? "");
    if (password !== String(form.get("confirm") ?? "")) {
      setError("The two passwords do not match.");
      return;
    }
    await run(async () => {
      await authPost("/reset-password", { token, newPassword: password });
      setDone(true);
    });
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
      <form className="signin-form" onSubmit={submitForm(submit)}>
        <label>
          <span>New password</span>
          <input
            type="password"
            name="password"
            placeholder="New password"
            autoComplete="new-password"
            minLength={8}
            required
          />
        </label>
        <label>
          <span>Repeat new password</span>
          <input
            type="password"
            name="confirm"
            placeholder="Repeat new password"
            autoComplete="new-password"
            minLength={8}
            required
          />
        </label>
        <button type="submit" className="btn" disabled={busy}>
          Save new password
        </button>
      </form>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
