import type { Metadata } from "next";
import ResetPasswordForm from "./reset-form";

export const metadata: Metadata = {
  title: "Reset password - Commy",
};

export const dynamic = "force-dynamic";

// The emailed reset link lands here with ?token=, or with ?error= when the
// link expired or was already used.
export default function ResetPasswordPage({
  searchParams,
}: {
  searchParams: { token?: string; error?: string };
}) {
  const token = (searchParams.token ?? "").slice(0, 256);

  return (
    <main className="page">
      <h1>Choose a new password</h1>
      {token && !searchParams.error ? (
        <ResetPasswordForm token={token} />
      ) : (
        <div className="account-card">
          <p className="form-error" role="alert">
            This reset link has expired or was already used.
          </p>
          <p className="muted">
            <a href="/account">Back to sign in</a> and choose Forgot password to get a new link.
          </p>
        </div>
      )}
    </main>
  );
}
