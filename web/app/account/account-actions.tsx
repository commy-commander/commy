"use client";

import { useState } from "react";
import { authPost } from "@/lib/auth-client";

export function SignOutButton() {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className="btn btn-danger"
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        await authPost("/sign-out").catch(() => undefined);
        window.location.href = "/";
      }}
    >
      Sign out
    </button>
  );
}

export function UnlinkDeviceButton({ id }: { id: string }) {
  const [busy, setBusy] = useState(false);
  return (
    <button
      type="button"
      className="link-button"
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        await authPost("/commy/devices/remove", { id }).catch(() => undefined);
        window.location.reload();
      }}
    >
      Unlink
    </button>
  );
}
