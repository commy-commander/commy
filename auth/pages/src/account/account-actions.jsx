import { useState } from "react";
import { authPost } from "../lib/auth-client.js";
import { Feedback, useSecureAction } from "./security-ui.jsx";

export function SignOutButton() {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className="btn btn-ghost sign-out"
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        await authPost("/sign-out").catch(() => undefined);
        window.location.href = "/account?notice=signed-out";
      }}
    >
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
        <path d="M12 4.35V11h7.44l-1.72-1.72a.75.75 0 0 1-.07-.98l.07-.08a.75.75 0 0 1 .98-.07l.08.07 3 3c.26.26.29.68.07.97l-.07.09-3 3a.75.75 0 0 1-1.13-.97l.07-.09 1.71-1.72H12v6.75c0 .47-.42.82-.88.74l-8.5-1.5a.75.75 0 0 1-.62-.74v-12c0-.37.27-.68.63-.74l8.5-1.4c.46-.07.87.28.87.74ZM8.5 11.5a1 1 0 1 0 0 2 1 1 0 0 0 0-2Zm4.5 7h.87a.75.75 0 0 0 .65-.75l-.01-4.25H13v5Zm0-8.5V5h.75c.37 0 .69.28.74.65v.1L14.5 10H13Z" />
      </svg>
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
    <>
      <button
        type="button"
        className="btn btn-ghost"
        disabled={busy}
        title="Signs out every other browser and desktop app"
        onClick={() =>
          run(async () => {
            await authPost("/revoke-other-sessions");
            window.location.href = "/account?notice=everywhere";
          })
        }
      >
        Sign out others
      </button>
      <Feedback error={error} stale={stale} />
    </>
  );
}
