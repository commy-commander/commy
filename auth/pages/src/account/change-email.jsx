import { useState } from "react";
import { authPost } from "../lib/auth-client.js";
import { Feedback, useSecureAction } from "./security-ui.jsx";
import { submitForm } from "../lib/forms.js";

export default function ChangeEmail({ currentEmail }) {
  const [open, setOpen] = useState(false);
  const { busy, error, stale, notice, setNotice, run } = useSecureAction();

  const submit = (form) =>
    run(async () => {
      const newEmail = String(form.get("newEmail") ?? "").trim();
      await authPost("/change-email", { newEmail, callbackURL: "/account?notice=email" });
      setOpen(false);
      setNotice(
        `Check ${currentEmail}. Confirm the move there, then open the last link we send to ${newEmail}. Your email stays the same until both are done.`,
      );
    });

  return (
    <div className="account-section">
      {open ? (
        <form className="signin-form" onSubmit={submitForm(submit)}>
          <label>
            <span>New email</span>
            <input type="email" name="newEmail" autoComplete="email" placeholder="you@example.com" required />
          </label>
          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={busy}>
              Send confirmation
            </button>
            <button type="button" className="btn btn-ghost" onClick={() => setOpen(false)} disabled={busy}>
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <button type="button" className="btn btn-ghost" onClick={() => setOpen(true)}>
          Change email
        </button>
      )}
      <Feedback error={error} stale={stale} notice={notice} />
    </div>
  );
}
