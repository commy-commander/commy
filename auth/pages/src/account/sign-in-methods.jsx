import { useState } from "react";
import { AuthError, authPost } from "../lib/auth-client.js";
import { submitForm } from "../lib/forms.js";
import { addPasskey, passkeyErrorMessage, passkeysWork } from "../lib/passkey.js";
import { Feedback, NewPasswordFields, PasswordField, readNewPassword, useSecureAction } from "./security-ui.jsx";
import { SOCIAL_ICONS, SOCIAL_LABELS } from "./sign-in-panel.jsx";

const ONLY_WAY_IN = "This is your only way to sign in. Add a password or another login before removing it.";

function PasswordMethod({ hasPassword }) {
  const [open, setOpen] = useState(false);
  const { busy, error, stale, notice, setNotice, run } = useSecureAction();

  const submit = (form) =>
    run(async () => {
      const newPassword = readNewPassword(form);
      if (hasPassword) {
        const signOutOthers = form.get("signOutOthers") === "on";
        await authPost("/change-password", {
          currentPassword: String(form.get("password") ?? ""),
          newPassword,
          revokeOtherSessions: signOutOthers,
        });
        setOpen(false);
        setNotice(signOutOthers ? "Password changed. Every other device is signed out." : "Password changed.");
        return;
      }
      await authPost("/commy/password/set", { newPassword });
      window.location.href = "/account?notice=password";
    });

  return (
    <li className="method">
      <div className="method-row">
        <span className="method-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <rect x="4" y="10" width="16" height="10" rx="2.5" />
            <path d="M8 10V7a4 4 0 0 1 8 0v3" />
          </svg>
        </span>
        <div className="method-text">
          <p>Password</p>
          <p className="muted small-line">{hasPassword ? "Set" : "Not set. You sign in with a link or another login."}</p>
        </div>
        {!open && (
          <button type="button" className="btn btn-ghost btn-compact" onClick={() => setOpen(true)}>
            {hasPassword ? "Change" : "Set a password"}
          </button>
        )}
      </div>
      {open && (
        <form className="signin-form method-form" onSubmit={submitForm(submit)}>
          {hasPassword && <PasswordField />}
          <NewPasswordFields />
          {hasPassword && (
            <label className="check-row">
              <input type="checkbox" name="signOutOthers" />
              <span>Also sign out every other browser and desktop app</span>
            </label>
          )}
          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={busy}>
              {hasPassword ? "Change password" : "Save password"}
            </button>
            <button type="button" className="btn btn-ghost" onClick={() => setOpen(false)} disabled={busy}>
              Cancel
            </button>
          </div>
        </form>
      )}
      <Feedback error={error} stale={stale} notice={notice} />
    </li>
  );
}

function SocialMethod({ provider, account, isLastWayIn }) {
  const { busy, error, stale, run } = useSecureAction();
  const label = SOCIAL_LABELS[provider];

  const link = () =>
    run(async () => {
      const res = await authPost("/link-social", {
        provider,
        callbackURL: "/account?notice=linked",
        errorCallbackURL: "/account",
      });
      if (res.url) window.location.href = res.url;
    });

  const unlink = () =>
    run(async () => {
      if (!account) return;
      await authPost("/unlink-account", { accountId: account.id });
      window.location.href = "/account?notice=unlinked";
    });

  return (
    <li className="method">
      <div className="method-row">
        <span className="method-icon" aria-hidden="true">
          {SOCIAL_ICONS[provider]}
        </span>
        <div className="method-text">
          <p>{label}</p>
          <p className="muted small-line">{account ? "Linked" : "Not linked"}</p>
        </div>
        {account ? (
          <button
            type="button"
            className="btn btn-ghost btn-compact"
            onClick={unlink}
            disabled={busy || isLastWayIn}
            title={isLastWayIn ? ONLY_WAY_IN : undefined}
          >
            Unlink
          </button>
        ) : (
          <button type="button" className="btn btn-ghost btn-compact" onClick={link} disabled={busy}>
            Link
          </button>
        )}
      </div>
      {account && isLastWayIn && <p className="muted small method-note">{ONLY_WAY_IN}</p>}
      <Feedback error={error} stale={stale} />
    </li>
  );
}

function PasskeyMethod({ passkeys }) {
  const [naming, setNaming] = useState(false);
  const { busy, error, setError, stale, run } = useSecureAction();

  const add = (form) =>
    run(async () => {
      try {
        await addPasskey(String(form.get("name") ?? "").trim());
      } catch (err) {
        throw err instanceof AuthError ? err : new Error(passkeyErrorMessage(err, "Could not add the passkey."));
      }
      window.location.href = "/account?notice=passkey";
    });

  const remove = (id) =>
    run(async () => {
      await authPost("/passkey/delete-passkey", { id });
      window.location.reload();
    });

  return (
    <li className="method">
      <div className="method-row">
        <span className="method-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="9" cy="8" r="4" />
            <path d="M2.5 20a6.5 6.5 0 0 1 10.6-5" />
            <circle cx="17.5" cy="15.5" r="2.5" />
            <path d="M17.5 18v3.5M17.5 20h1.8" />
          </svg>
        </span>
        <div className="method-text">
          <p>Passkeys</p>
          <p className="muted small-line">
            {passkeys.length === 0 ? "Sign in with your fingerprint, face or device PIN." : `${passkeys.length} added`}
          </p>
        </div>
        {!naming && (
          <button
            type="button"
            className="btn btn-ghost btn-compact"
            onClick={() => (passkeysWork() ? setNaming(true) : setError("This browser cannot create passkeys."))}
            disabled={busy}
          >
            Add a passkey
          </button>
        )}
      </div>
      {passkeys.length > 0 && (
        <ul className="passkey-list">
          {passkeys.map((key) => (
            <li key={key.id}>
              <span>
                {key.name || "Passkey"}
                {key.createdAt && (
                  <span className="muted"> · added {new Date(key.createdAt).toLocaleDateString("en-US")}</span>
                )}
              </span>
              <button type="button" className="link-button" onClick={() => remove(key.id)} disabled={busy}>
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      {naming && (
        <form className="signin-form method-form" onSubmit={submitForm(add)}>
          <label>
            <span>Name (optional)</span>
            <input type="text" name="name" maxLength={40} placeholder="For example, Laptop or Phone" />
          </label>
          <div className="form-actions">
            <button type="submit" className="btn btn-primary" disabled={busy}>
              Create passkey
            </button>
            <button type="button" className="btn btn-ghost" onClick={() => setNaming(false)} disabled={busy}>
              Cancel
            </button>
          </div>
        </form>
      )}
      <Feedback error={error} stale={stale} />
    </li>
  );
}

export default function SignInMethods({
  accounts,
  passkeys,
  socials,
  passkeysOffered,
}) {
  const hasPassword = accounts.some((a) => a.providerId === "credential");
  const isLastWayIn = accounts.length <= 1;
  const shown = ["google", "twitch"].filter(
    (p) => socials.includes(p) || accounts.some((a) => a.providerId === p),
  );

  return (
    <ul className="method-list">
      <PasswordMethod hasPassword={hasPassword} />
      {shown.map((p) => (
        <SocialMethod key={p} provider={p} account={accounts.find((a) => a.providerId === p)} isLastWayIn={isLastWayIn} />
      ))}
      {passkeysOffered && <PasskeyMethod passkeys={passkeys} />}
    </ul>
  );
}
