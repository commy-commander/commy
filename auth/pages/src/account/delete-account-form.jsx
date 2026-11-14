import { useState } from "react";
import { authPost } from "../lib/auth-client.js";
import { DELETE_PAGE, deletionProof, startMode } from "../lib/account-deletion.js";
import { Feedback, useSecureAction } from "./security-ui.jsx";

function sentLine(start) {
  if (start.confirm === "email") return `We emailed a 6-digit code to ${start.email ?? "your address"}. It expires in 15 minutes.`;
  if (start.password) return "Enter your password to confirm.";
  return "This server does not send email, so your recent sign-in confirms this.";
}

export default function DeleteAccountForm({ email }) {
  const [start, setStart] = useState(null);
  const [mode, setMode] = useState("code");
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const { busy, error, stale, run } = useSecureAction();
  const proof = deletionProof(mode, code, password);

  const sendCode = () =>
    run(async () => {
      const answer = await authPost("/commy/account/delete/start");
      setStart(answer);
      setMode(startMode(answer));
    });

  const deleteNow = () =>
    run(async () => {
      if (!proof) return;
      await authPost("/commy/account/delete/finish", proof);
      window.location.href = `${DELETE_PAGE}?notice=deleted`;
    });

  if (!start) {
    return (
      <div className="account-section">
        <p>
          You are signed in as <strong>{email}</strong>.
        </p>
        <div className="form-actions">
          <button type="button" className="btn btn-danger" disabled={busy} onClick={sendCode}>
            Start deleting my account
          </button>
        </div>
        <Feedback error={error} stale={stale} />
      </div>
    );
  }

  return (
    <form
      className="signin-form"
      onSubmit={(event) => {
        event.preventDefault();
        deleteNow();
      }}
    >
      <p className="muted">{mode === "password" ? "Enter your password to confirm." : sentLine(start)}</p>
      {mode === "code" && (
        <label>
          <span>Code from the email</span>
          <input
            name="code"
            inputMode="numeric"
            autoComplete="one-time-code"
            maxLength={9}
            value={code}
            onChange={(event) => setCode(event.target.value)}
            required
          />
        </label>
      )}
      {mode === "password" && (
        <label>
          <span>Your password</span>
          <input
            type="password"
            name="password"
            autoComplete="current-password"
            maxLength={128}
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            required
          />
        </label>
      )}
      <div className="form-actions">
        <button type="submit" className="btn btn-danger" disabled={busy || !proof}>
          Delete my account for good
        </button>
        {mode === "code" && start.password && (
          <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => setMode("password")}>
            Use my password instead
          </button>
        )}
        {mode === "code" && (
          <button type="button" className="btn btn-ghost" disabled={busy} onClick={sendCode}>
            Send a new code
          </button>
        )}
      </div>
      <Feedback error={error} stale={stale} />
    </form>
  );
}
