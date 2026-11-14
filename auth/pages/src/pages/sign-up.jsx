import AccessGate from "../account/access-gate.jsx";
import SignInPanel from "../account/sign-in-panel.jsx";
import { signUpHref } from "../lib/safe-next.js";
import { useSite } from "../site.jsx";

export default function SignUpPage({ locked, providers, next }) {
  const { name } = useSite();
  if (locked) {
    return (
      <main className="page auth-page">
        <h1>Create your {name} account</h1>
        <p className="muted">{name} is in private preview. Enter the access code you were given to create an account.</p>
        <div className="account-card">
          <AccessGate next={signUpHref(next)} />
          <p className="muted small">
            Already have an account? <a href={next}>Log in</a>
          </p>
        </div>
      </main>
    );
  }
  return (
    <main className="page auth-page">
      <h1>Create your {name} account</h1>
      <p className="muted">
        One account links your identity across devices. It never sees your messages, calls, or files.
      </p>
      <SignInPanel providers={providers} callbackURL={next} initialMode="signup" />
    </main>
  );
}
