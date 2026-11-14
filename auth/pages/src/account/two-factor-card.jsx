import { useMemo, useState } from "react";
import { encode } from "uqr";
import { authPost } from "../lib/auth-client.js";
import { Feedback, PasswordField, useSecureAction } from "./security-ui.jsx";
import { submitForm } from "../lib/forms.js";

function QrCode({ text }) {
  const { data, size } = useMemo(() => encode(text, { ecc: "M", border: 2 }), [text]);
  const cells = [];
  data.forEach((row, y) =>
    row.forEach((dark, x) => {
      if (dark) cells.push(`M${x} ${y}h1v1h-1z`);
    }),
  );
  return (
    <svg className="qr" viewBox={`0 0 ${size} ${size}`} role="img" aria-label="QR code for your authenticator app">
      <rect width={size} height={size} fill="#ffffff" />
      <path d={cells.join("")} fill="#0b0d17" />
    </svg>
  );
}

function secretOf(totpURI) {
  const secret = new URL(totpURI).searchParams.get("secret") ?? "";
  return secret.replace(/(.{4})/g, "$1 ").trim();
}

function BackupCodes({ codes, onDone }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="method-form">
      <p>
        Save these backup codes somewhere safe, like a password manager. Each one signs you in once if you lose your
        authenticator. They will not be shown again.
      </p>
      <ul className="backup-codes">
        {codes.map((code) => (
          <li key={code}>{code}</li>
        ))}
      </ul>
      <div className="form-actions">
        <button
          type="button"
          className="btn btn-ghost"
          onClick={() =>
            navigator.clipboard
              .writeText(codes.join("\n"))
              .then(() => setCopied(true))
              .catch(() => undefined)
          }
        >
          {copied ? "Copied" : "Copy codes"}
        </button>
        <button type="button" className="btn btn-primary" onClick={onDone}>
          I saved them
        </button>
      </div>
    </div>
  );
}

export default function TwoFactorCard({ enabled, hasPassword }) {
  const [step, setStep] = useState("idle");
  const [setup, setSetup] = useState(null);
  const [codes, setCodes] = useState([]);
  const { busy, error, stale, run } = useSecureAction();
  const password = (form) => (hasPassword ? { password: String(form.get("password") ?? "") } : {});

  const start = (form) =>
    run(async () => {
      setSetup(await authPost("/two-factor/enable", password(form)));
      setStep("scan");
    });

  const confirm = (form) =>
    run(async () => {
      await authPost("/two-factor/verify-totp", { code: String(form.get("code") ?? "").replace(/\s+/g, "") });
      setCodes(setup?.backupCodes ?? []);
      setStep("codes");
    });

  const turnOff = (form) =>
    run(async () => {
      await authPost("/two-factor/disable", password(form));
      window.location.href = "/account?notice=two-factor-off";
    });

  const newCodes = (form) =>
    run(async () => {
      const res = await authPost("/two-factor/generate-backup-codes", password(form));
      setCodes(res.backupCodes);
      setStep("codes");
    });

  const cancel = () => {
    setStep("idle");
    setSetup(null);
  };

  const askPassword = (action, submitLabel, danger = false) => (
    <form className="signin-form method-form" onSubmit={submitForm(action)}>
      {hasPassword && <PasswordField />}
      <div className="form-actions">
        <button type="submit" className={danger ? "btn btn-danger" : "btn btn-primary"} disabled={busy}>
          {submitLabel}
        </button>
        <button type="button" className="btn btn-ghost" onClick={cancel} disabled={busy}>
          Cancel
        </button>
      </div>
    </form>
  );

  const finished = () => {
    window.location.href = enabled ? "/account?notice=backup-codes" : "/account?notice=two-factor-on";
  };

  return (
    <div className="account-card">
      <h2>Two-step sign-in</h2>
      <p className="muted">
        {enabled
          ? "On. Signing in with a password, email link, Google or Twitch also asks for a code from your authenticator app."
          : "Ask for a code from an authenticator app whenever you sign in with a password, email link, Google or Twitch. Passkeys skip it, since they already prove it's you."}
      </p>

      {step === "idle" && (
        <div className="form-actions section-actions">
          {enabled ? (
            <>
              <button type="button" className="btn btn-ghost" onClick={() => setStep("confirm-codes")}>
                New backup codes
              </button>
              <button type="button" className="btn btn-danger" onClick={() => setStep("confirm-off")}>
                Turn off
              </button>
            </>
          ) : (
            <button type="button" className="btn btn-primary" onClick={() => setStep("confirm-on")}>
              Turn on
            </button>
          )}
        </div>
      )}

      {step === "confirm-on" && askPassword(start, "Continue")}
      {step === "confirm-off" && askPassword(turnOff, "Turn off two-step sign-in", true)}
      {step === "confirm-codes" && askPassword(newCodes, "Replace my backup codes")}

      {step === "scan" && setup && (
        <form className="signin-form method-form" onSubmit={submitForm(confirm)}>
          <p>Scan this with an authenticator app such as your password manager, then enter the 6-digit code it shows.</p>
          <div className="qr-row">
            <QrCode text={setup.totpURI} />
            <div>
              <p className="muted small-line">Can&apos;t scan? Enter this key instead:</p>
              <p className="device-node">{secretOf(setup.totpURI)}</p>
            </div>
          </div>
          <label>
            <span>Code from the app</span>
            <input
              className="code-input"
              name="code"
              inputMode="numeric"
              autoComplete="one-time-code"
              pattern="[0-9 ]{6,7}"
              maxLength={7}
              required
            />
          </label>
          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={busy}>
              Turn on
            </button>
            <button type="button" className="btn btn-ghost" onClick={cancel} disabled={busy}>
              Cancel
            </button>
          </div>
        </form>
      )}

      {step === "codes" && <BackupCodes codes={codes} onDone={finished} />}

      <Feedback error={error} stale={stale} />
    </div>
  );
}
