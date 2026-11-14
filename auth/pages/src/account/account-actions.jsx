import { useState } from "react";
import { authPost } from "../lib/auth-client.js";
import { useSite } from "../site.jsx";
import { Feedback, useSecureAction } from "./security-ui.jsx";

export function SignOutButton() {
  const { links } = useSite();
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className="btn btn-danger"
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        await authPost("/sign-out").catch(() => undefined);
        window.location.href = links.home;
      }}
    >
      Sign out
    </button>
  );
}

export function UnlinkDeviceButton({ id }) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className="link-button"
      disabled={busy}
      title="Removes this app from your account and signs it out"
      onClick={async () => {
        setBusy(true);
        await authPost("/commy/devices/remove", { id }).catch(() => undefined);
        window.location.href = "/account?notice=device";
      }}
    >
      Unlink
    </button>
  );
}

export function SignOutEverywhereElse() {
  const { busy, error, stale, run } = useSecureAction();
  return (
    <div className="account-section">
      <button
        type="button"
        className="btn btn-ghost"
        disabled={busy}
        onClick={() =>
          run(async () => {
            await authPost("/revoke-other-sessions");
            window.location.href = "/account?notice=everywhere";
          })
        }
      >
        Sign out everywhere else
      </button>
      <p className="muted small">Signs out every other browser and every desktop app, including ones linked before.</p>
      <Feedback error={error} stale={stale} />
    </div>
  );
}
