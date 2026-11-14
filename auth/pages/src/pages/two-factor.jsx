import TwoFactorForm from "../account/two-factor-form.jsx";

export default function TwoFactorPage({ next }) {
  return (
    <main className="page auth-page">
      <h1>Two-step sign-in</h1>
      <p className="muted">Enter the 6-digit code from your authenticator app to finish signing in.</p>
      <TwoFactorForm next={next} />
    </main>
  );
}
