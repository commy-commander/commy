import ResetPasswordForm from "../account/reset-form.jsx";

export default function ResetPasswordPage({ token }) {
  return (
    <main className="page auth-page">
      <h1>Choose a new password</h1>
      {token ? (
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
