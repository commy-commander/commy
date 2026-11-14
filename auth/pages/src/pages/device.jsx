import DeviceApproval from "../account/device-approval.jsx";
import SignInPanel from "../account/sign-in-panel.jsx";

export default function DevicePage({ code, email, providers, returnTo }) {
  return (
    <main className="page">
      <h1>Sign in to the Commy app</h1>
      {email ? (
        <DeviceApproval initialCode={code} userEmail={email} />
      ) : (
        <>
          <p className="muted">Sign in first, then confirm the code shown in the app.</p>
          <SignInPanel providers={providers} callbackURL={returnTo} />
        </>
      )}
    </main>
  );
}
