import { useState } from "react";
import { AuthError, authPost } from "../lib/auth-client.js";
import { submitForm } from "../lib/forms.js";
import CardHead from "./card-head.jsx";
import { addPasskey, passkeyErrorMessage, passkeysWork } from "../lib/passkey.js";
import { Feedback, NewPasswordFields, PasswordField, readNewPassword, useSecureAction } from "./security-ui.jsx";
import { SOCIAL_ICONS, SOCIAL_LABELS } from "./sign-in-panel.jsx";

const ONLY_WAY_IN = "This is your only way to sign in. Add a password or another login before removing it.";

export function PasswordCard({ hasPassword, email, canReset }) {
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

  const sendReset = () =>
    run(async () => {
      await authPost("/request-password-reset", {
        email,
        redirectTo: `${window.location.origin}/account/reset-password`,
      });
      setOpen(false);
      setNotice(`We sent a reset link to ${email}. It expires in 1 hour.`);
    });

  return (
    <section className="account-card password-card" aria-labelledby="password-title">
      <CardHead
        icon="lock"
        id="password-title"
        title="Password"
        action={
          !open && (
            <button type="button" className="btn btn-primary" onClick={() => setOpen(true)}>
              {hasPassword ? "Change" : "Set password"}
            </button>
          )
        }
      >
        <p className="muted">{hasPassword ? "Sign in with your email and password." : "Not set yet."}</p>
      </CardHead>
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
          {hasPassword && canReset && (
            <p className="muted small form-aside">
              Forgot your current password?{" "}
              <button type="button" className="link-button" onClick={sendReset} disabled={busy}>
                Email me a reset link
              </button>
            </p>
          )}
        </form>
      )}
      <Feedback error={error} stale={stale} notice={notice} />
    </section>
  );
}

function SocialMethod({ provider, account, isLastWayIn, emailSignIn }) {
  const [confirming, setConfirming] = useState(false);
  const { busy, error, stale, run } = useSecureAction();
  const label = SOCIAL_LABELS[provider];
  const locked = isLastWayIn && !emailSignIn;

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
          !confirming && (
            <button
              type="button"
              className="btn btn-ghost btn-compact"
              onClick={isLastWayIn ? () => setConfirming(true) : unlink}
              disabled={busy || locked}
              title={locked ? ONLY_WAY_IN : undefined}
            >
              Unlink
            </button>
          )
        ) : (
          <button type="button" className="btn btn-ghost btn-compact" onClick={link} disabled={busy}>
            Link
          </button>
        )}
      </div>
      {account && locked && <p className="muted small method-note">{ONLY_WAY_IN}</p>}
      {account && confirming && (
        <div className="method-form method-confirm">
          <p className="muted small">
            {label} is your only login. After you unlink it, you sign in with a link we email to you, or set a password first.
          </p>
          <div className="form-actions">
            <button type="button" className="btn btn-danger" onClick={unlink} disabled={busy}>
              Unlink {label}
            </button>
            <button type="button" className="btn btn-ghost" onClick={() => setConfirming(false)} disabled={busy}>
              Cancel
            </button>
          </div>
        </div>
      )}
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
          <svg viewBox="0 0 24 24" fill="currentColor">
            <path d="M15 15.5c0-.53.1-1.03.26-1.5h-9C5 14 4 15 4 16.25v.92c0 .57.18 1.13.51 1.6 1.4 1.94 3.57 3 6.49 3.2V20.9a2 2 0 0 1 .59-1.41l3.44-3.45a4.54 4.54 0 0 1-.03-.55ZM12 2a5 5 0 1 1 0 10 5 5 0 0 1 0-10Zm7.5 17a3.5 3.5 0 1 0-3.39-2.61L12.3 20.2a1 1 0 0 0-.29.7v1.59c0 .28.22.5.5.5h2a.5.5 0 0 0 .5-.5V22h1a.5.5 0 0 0 .5-.5v-1h1a.5.5 0 0 0 .5-.5v-1h1.5Zm.5-5a1 1 0 1 1 0 2 1 1 0 0 1 0-2Z" />
          </svg>
        </span>
        <div className="method-text">
          <p>Passkeys</p>
          <p className="muted small-line">
            {passkeys.length === 0 ? "Fingerprint, face or device PIN." : `${passkeys.length} added`}
          </p>
        </div>
        {!naming && (
          <button
            type="button"
            className="btn btn-ghost btn-compact"
            onClick={() => (passkeysWork() ? setNaming(true) : setError("This browser cannot create passkeys."))}
            disabled={busy}
          >
            Add
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
  emailSignIn,
}) {
  const isLastWayIn = accounts.length <= 1;
  const shown = ["google", "twitch"].filter(
    (p) => socials.includes(p) || accounts.some((a) => a.providerId === p),
  );

  return (
    <ul className="method-list">
      {shown.map((p) => (
        <SocialMethod key={p} provider={p} account={accounts.find((a) => a.providerId === p)} isLastWayIn={isLastWayIn} emailSignIn={emailSignIn} />
      ))}
      {passkeysOffered && <PasskeyMethod passkeys={passkeys} />}
    </ul>
  );
}
