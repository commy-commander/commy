import { SignOutButton, SignOutEverywhereElse, UnlinkDeviceButton } from "../account/account-actions.jsx";
import ChangeEmail from "../account/change-email.jsx";
import SignInMethods from "../account/sign-in-methods.jsx";
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

function SignedOut({ providers, next, errorMessage }) {
  const { name } = useSite();
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

function Profile({ user, changeEmail }) {
  const displayName = user.name || "Commy user";
  return (
    <div className="account-card">
      <h2>Profile</h2>
      <div className="profile-row">
        <div className="avatar">{user.image ? <img src={user.image} alt="" /> : displayName.charAt(0).toUpperCase()}</div>
        <div>
          <p>{displayName}</p>
          <p className="muted">{user.email}</p>
        </div>
      </div>
      {changeEmail && <ChangeEmail currentEmail={user.email} />}
    </div>
  );
}

function LinkedDevices({ devices }) {
  return (
    <div className="account-card">
      <h2>Linked devices</h2>
      {devices.length > 0 ? (
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
      ) : (
        <p className="muted">
          No devices linked yet. Install the Commy desktop app and choose Sign
          in under Settings to link it to this account.
        </p>
      )}
      <SignOutEverywhereElse />
    </div>
  );
}

function SignedIn({ user, providers, accounts, passkeys, devices, errorMessage, notice }) {
  const socials = ["google", "twitch"].filter((p) => providers[p]);
  const hasPassword = accounts.some((a) => a.providerId === "credential");
  return (
    <main className="page">
      <h1>Your account</h1>
      <Messages errorMessage={errorMessage} notice={notice} />
      <Profile user={user} changeEmail={providers.changeEmail} />

      <div className="account-card">
        <h2>Ways to sign in</h2>
        <SignInMethods accounts={accounts} passkeys={passkeys} socials={socials} passkeysOffered={providers.passkey} />
      </div>

      {providers.twoFactor && <TwoFactorCard enabled={Boolean(user.twoFactorEnabled)} hasPassword={hasPassword} />}

      <LinkedDevices devices={devices} />

      <div className="account-card">
        <h2>Delete account</h2>
        <p className="muted">
          Deletes your account, every way you sign in, your linked devices, invite links and Discover listings for good.
        </p>
        <div className="account-section">
          <a className="btn btn-ghost" href="/delete-account">
            Delete my account
          </a>
        </div>
      </div>

      <SignOutButton />
    </main>
  );
}

export default function AccountPage(props) {
  return props.user ? <SignedIn {...props} /> : <SignedOut {...props} />;
}
