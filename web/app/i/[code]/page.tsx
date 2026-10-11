import type { Metadata } from "next";
import Link from "next/link";
import { headers } from "next/headers";
import { resolveInvite } from "@/lib/auth";
import InviteActions from "./invite-actions";

export const dynamic = "force-dynamic";

type Props = { params: { code: string } };

function cleanCode(raw: string) {
  return raw.toUpperCase().replace(/[^A-Z0-9]/g, "").slice(0, 16);
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const invite = await resolveInvite(cleanCode(params.code));
  if (!invite) return { title: "Invite not found - Commy" };
  const title = `Join ${invite.serverName} on Commy`;
  return {
    title,
    description: "You have been invited to a Commy server. Commy is free, open source, peer to peer chat and voice.",
    openGraph: { title, siteName: "Commy" },
    robots: { index: false },
  };
}

export default async function InvitePage({ params }: Props) {
  const code = cleanCode(params.code);
  const invite = code ? await resolveInvite(code) : null;
  const host = headers().get("host") ?? "";

  if (!invite) {
    return (
      <main className="page invite-page">
        <h1>Invite not found</h1>
        <p className="muted">This invite is invalid or has expired. Ask whoever sent it for a new one.</p>
        <Link href="/" className="btn">
          Go to Commy
        </Link>
      </main>
    );
  }

  const expires = new Date(invite.expiresAt).toLocaleDateString("en-US", { month: "long", day: "numeric" });

  return (
    <main className="page invite-page">
      <div className="account-card invite-hero">
        <p className="muted">You have been invited to join</p>
        <h1 className="invite-name">{invite.serverName}</h1>
        <p className="muted small">
          Invite hosted by {host} · expires {expires}
        </p>
        <InviteActions host={host} code={invite.code} url={invite.url} />
      </div>
      <p className="muted small">
        Commy servers are peer to peer: joining connects you directly to the members who are online, and nothing
        you send passes through {host}. Only join servers from people you trust.
      </p>
    </main>
  );
}
