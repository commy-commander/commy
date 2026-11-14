import DeleteAccountForm from "../account/delete-account-form.jsx";
import SignInPanel from "../account/sign-in-panel.jsx";
import { DELETE_PAGE, DELETED_RIGHT_AWAY, NOT_DELETED } from "../lib/account-deletion.js";
import { useSite } from "../site.jsx";

function Lines({ lines }) {
  return (
    <ul className="plain-list">
      {lines.map((line) => (
        <li key={line}>{line}</li>
      ))}
    </ul>
  );
}

function DeleteHere({ email, providers }) {
  return (
    <div className="account-card">
      <h2>Delete it here</h2>
      {email ? (
        <DeleteAccountForm email={email} />
      ) : (
        <>
          <p className="muted">Sign in to the account you want to delete.</p>
          {providers && <SignInPanel providers={providers} callbackURL={DELETE_PAGE} />}
        </>
      )}
    </div>
  );
}

export default function DeleteAccountPage({ notice, email, providers }) {
  const { name } = useSite();
  return (
    <main className="page">
      <h1>Delete your {name} account</h1>
      {notice && (
        <p className="form-notice" role="status">
          {notice}
        </p>
      )}
      <p className="muted">
        You can delete your account in any Commy app, or here without the app. You confirm with a code we email you,
        or with your password.
      </p>

      <div className="account-card">
        <h2>In the app</h2>
        <p>
          Open Settings, choose Commy Account, then Delete Account. It works the same in the desktop app, the phone app
          and the browser app.
        </p>
      </div>

      <div className="account-card">
        <h2>Deleted right away</h2>
        <Lines lines={DELETED_RIGHT_AWAY} />
      </div>

      <div className="account-card">
        <h2>What is not deleted</h2>
        <Lines lines={NOT_DELETED} />
      </div>

      {!notice && <DeleteHere email={email} providers={providers} />}
    </main>
  );
}
