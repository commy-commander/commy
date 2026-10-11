import type { Metadata } from "next";
import { getProviders, getSessionUser, listDevices } from "@/lib/auth";
import SignInPanel from "./sign-in-panel";
import { SignOutButton, UnlinkDeviceButton } from "./account-actions";

export const metadata: Metadata = {
  title: "Account - Commy",
};

export const dynamic = "force-dynamic";

export default async function AccountPage() {
  const user = await getSessionUser();

  if (!user) {
    const providers = await getProviders();
    return (
      <main className="page">
        <h1>Sign in to Commy</h1>
        <p className="muted">
          Your account links your identity across devices. It never sees your
          messages, calls, or files.
        </p>
        <SignInPanel providers={providers} callbackURL="/account" />
      </main>
    );
  }

  const devices = await listDevices();
  const displayName = user.name || "Commy user";
  const initial = displayName.charAt(0).toUpperCase();

  return (
    <main className="page">
      <h1>Your account</h1>

      <div className="account-card">
        <h2>Profile</h2>
        <div className="profile-row">
          <div className="avatar">
            {user.image ? (
              // eslint-disable-next-line @next/next/no-img-element
              <img src={user.image} alt="" />
            ) : (
              initial
            )}
          </div>
          <div>
            <p>{displayName}</p>
            <p className="muted">{user.email}</p>
          </div>
        </div>
      </div>

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
      </div>

      <SignOutButton />
    </main>
  );
}
