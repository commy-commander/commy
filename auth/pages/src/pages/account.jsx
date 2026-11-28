import { SignOutButton, SignOutEverywhereElse, UnlinkDeviceButton } from "../account/account-actions.jsx";
import CardHead from "../account/card-head.jsx";
import ChangeEmail from "../account/change-email.jsx";
import SignInMethods, { PasswordCard } from "../account/sign-in-methods.jsx";
import SignInPanel from "../account/sign-in-panel.jsx";
import TwoFactorCard from "../account/two-factor-card.jsx";
import { useSite } from "../site.jsx";

function Messages({ errorMessage, notice }) {
  return (
    <>
      {errorMessage && (
        <p className="form-error" role="alert">
          {errorMessage}
        </p>
      )}
      {notice && (
        <p className="form-notice" role="status">
          {notice}
        </p>
      )}
    </>
  );
}

function SignedOutNotice() {
  const { name, links } = useSite();
  return (
    <main className="page signed-out-page">
      <div className="account-card signed-out-card">
        <span className="signed-out-mark" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="currentColor">
            <path d="m8.5 16.59-3.8-3.8a1 1 0 0 0-1.4 1.42l4.5 4.5a1 1 0 0 0 1.4 0l11-11a1 1 0 0 0-1.4-1.42L8.5 16.6Z" />
          </svg>
        </span>
        <h1>You are signed out</h1>
        <p className="muted">
          This browser is signed out of {name}. Your desktop and phone apps stay signed in until you sign them out there.
        </p>
        <div className="signed-out-actions">
          <a className="btn btn-primary" href="/account">
            Sign in again
          </a>
          <a className="btn btn-ghost" href={links.home || "/"}>
            Back to {name}
          </a>
        </div>
      </div>
    </main>
  );
}

function SignedOut({ providers, next, errorMessage, signedOut }) {
  const { name } = useSite();
  if (signedOut && !errorMessage) return <SignedOutNotice />;
  return (
    <main className="page auth-page">
      <h1>Sign in to {name}</h1>
      <p className="muted">
        Your account links your identity across devices. It never sees your
        messages, calls, or files.
      </p>
      <Messages errorMessage={errorMessage} />
      <SignInPanel providers={providers} callbackURL={next} />
    </main>
  );
}

function AccountHead({ user }) {
  const displayName = user.name || "Commy user";
  return (
    <header className="account-head">
      <h1 className="account-title">Your account</h1>
      <div className="account-card account-id">
        <div className="avatar avatar-lg">{user.image ? <img src={user.image} alt="" /> : displayName.charAt(0).toUpperCase()}</div>
        <div className="account-who">
          <p className="account-name">{displayName}</p>
          <p className="muted">{user.email}</p>
        </div>
        <SignOutButton />
      </div>
    </header>
  );
}

function LinkedDevices({ devices }) {
  return (
    <section className="account-card" aria-labelledby="devices-title">
      <CardHead icon="devices" id="devices-title" title="Linked devices" action={<SignOutEverywhereElse />}>
        <p className="muted">{devices.length > 0 ? `${devices.length} linked` : "Sign in from the desktop app to link it."}</p>
      </CardHead>
      {devices.length > 0 && (
        <ul className="device-list">
          {devices.map((device) => (
            <li key={device.id}>
              <div>
                <p>{device.name}</p>
                <p className="device-node">{device.nodeId}</p>
              </div>
              <span className="muted">
                Last seen {new Date(device.lastSeenAt).toLocaleDateString("en-US")}
                {" · "}
                <UnlinkDeviceButton id={device.id} />
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function SignedIn({ user, providers, accounts, passkeys, devices, errorMessage, notice }) {
  const socials = ["google", "twitch"].filter((p) => providers[p]);
  const hasPassword = accounts.some((a) => a.providerId === "credential");
  const otherWays = providers.passkey || ["google", "twitch"].some((p) => socials.includes(p) || accounts.some((a) => a.providerId === p));
  return (
    <main className="page account-page">
      <AccountHead user={user} />
      <Messages errorMessage={errorMessage} notice={notice} />

      <div className="account-cols">
        <div className="account-col">
          <PasswordCard hasPassword={hasPassword} email={user.email} canReset={providers.passwordReset} />
          {otherWays && (
            <section className="account-card" aria-labelledby="ways-title">
              <CardHead icon="key" id="ways-title" title="Other ways to sign in" />
              <SignInMethods accounts={accounts} passkeys={passkeys} socials={socials} passkeysOffered={providers.passkey} emailSignIn={providers.magicLink} />
            </section>
          )}
        </div>
        <div className="account-col">
          {providers.changeEmail && <ChangeEmail currentEmail={user.email} />}
          {providers.twoFactor && <TwoFactorCard enabled={Boolean(user.twoFactorEnabled)} hasPassword={hasPassword} />}
          <LinkedDevices devices={devices} />
          <section className="account-card danger-card" aria-labelledby="delete-title">
            <CardHead
              icon="trash"
              id="delete-title"
              title="Delete account"
              action={
                <a className="btn btn-danger" href="/delete-account">
                  Delete
                </a>
              }
            >
              <p className="muted">Removes it and everything tied to it for good.</p>
            </CardHead>
          </section>
        </div>
      </div>
    </main>
  );
}

export default function AccountPage(props) {
  return props.user ? <SignedIn {...props} /> : <SignedOut {...props} />;
}
