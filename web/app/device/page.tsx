import type { Metadata } from "next";
import { getProviders, getSessionUser } from "@/lib/auth";
import SignInPanel from "../account/sign-in-panel";
import DeviceApproval from "./device-approval";

export const metadata: Metadata = {
  title: "Sign in a device - Commy",
};

export const dynamic = "force-dynamic";

// Shown when the desktop app asks to sign in: it opens this page with the
// code it is displaying, and the signed in user approves it here.
export default async function DevicePage({
  searchParams,
}: {
  searchParams: { user_code?: string };
}) {
  const code = (searchParams.user_code ?? "").toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 16);
  const user = await getSessionUser();
  const returnTo = code ? `/device?user_code=${code}` : "/device";

  return (
    <main className="page">
      <h1>Sign in to the Commy app</h1>
      {user ? (
        <DeviceApproval initialCode={code} userEmail={user.email} />
      ) : (
        <>
          <p className="muted">Sign in first, then confirm the code shown in the app.</p>
          <SignInPanel providers={await getProviders()} callbackURL={returnTo} />
        </>
      )}
    </main>
  );
}
