"use client";

import { useEffect, useState } from "react";
import { authGet, authPost } from "@/lib/auth-client";
import type { DeviceRequestContext } from "@/lib/auth";

type Stage = "enter" | "review" | "approved" | "denied";

export default function DeviceApproval({ initialCode, userEmail }: { initialCode: string; userEmail: string }) {
  const [code, setCode] = useState(initialCode);
  const [stage, setStage] = useState<Stage>("enter");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [context, setContext] = useState<DeviceRequestContext | null>(null);

  async function lookup(userCode: string) {
    setBusy(true);
    setError("");
    try {
      // Looking the code up while signed in claims it for this account
      const res = await authGet<{ status: string }>(`/device?user_code=${encodeURIComponent(userCode)}`);
      if (res.status !== "pending") {
        setError("This code has already been used. Start sign in again from the app.");
        return;
      }
      // Called from the browser so the server compares this network, not the website's
      setContext(
        await authGet<DeviceRequestContext>(`/commy/device-request?user_code=${encodeURIComponent(userCode)}`).catch(
          () => null,
        ),
      );
      setStage("review");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    if (initialCode) void lookup(initialCode);
  }, [initialCode]);

  async function decide(approve: boolean) {
    setBusy(true);
    setError("");
    try {
      await authPost(approve ? "/device/approve" : "/device/deny", { userCode: code });
      setStage(approve ? "approved" : "denied");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="account-card">
      {stage === "enter" && (
        <form
          className="signin-form"
          onSubmit={(e) => {
            e.preventDefault();
            void lookup(code);
          }}
        >
          <p>Enter the code shown in the Commy app.</p>
          <input
            className="code-input"
            value={code}
            onChange={(e) => setCode(e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, ""))}
            placeholder="ABCD1234"
            maxLength={16}
            autoFocus
            required
          />
          <button type="submit" className="btn btn-primary" disabled={busy}>
            Continue
          </button>
        </form>
      )}

      {stage === "review" && (
        <>
          <p>
            The Commy app showing code <strong className="device-node">{code}</strong> wants to sign in as{" "}
            <strong>{userEmail}</strong>.
          </p>
          {context?.requestedAt && <p className="muted">Requested {minutesAgo(context.requestedAt)}.</p>}
          {context?.sameNetwork === false && (
            <div className="warning-box" role="alert">
              <strong>This request came from a different network than the one you are on now.</strong> If someone sent
              you this link or code, deny it: approving would give them your account.
            </div>
          )}
          <p className="muted">
            Only approve if you just clicked Sign in in the Commy app on a computer in front of you. Commy staff will
            never ask you to approve a code.
          </p>
          <div className="hero-actions">
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => decide(true)}>
              Approve
            </button>
            <button type="button" className="btn" disabled={busy} onClick={() => decide(false)}>
              Deny
            </button>
          </div>
        </>
      )}

      {stage === "approved" && <p>You are signed in. You can close this tab and go back to Commy.</p>}
      {stage === "denied" && <p>Request denied. The app was not signed in.</p>}

      {error && <p className="form-error" role="alert">{error}</p>}
    </div>
  );
}

function minutesAgo(iso: string) {
  const minutes = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 60000));
  return minutes === 0 ? "just now" : minutes === 1 ? "1 minute ago" : `${minutes} minutes ago`;
}
