import InviteActions from "../account/invite-actions.jsx";
import { useSite } from "../site.jsx";

function InviteNotFound() {
  const { name, links } = useSite();
  return (
    <main className="page invite-page">
      <h1>Invite not found</h1>
      <p className="muted">This invite is invalid or has expired. Ask whoever sent it for a new one.</p>
      <a href={links.home} className="btn">
        Go to {name}
      </a>
    </main>
  );
}

export default function InvitePage({ invite }) {
  const { host } = useSite();
  if (!invite) return <InviteNotFound />;
  return (
    <main className="page invite-page">
      <div className="account-card invite-hero">
        <InviteActions
          host={host}
          code={invite.code}
          url={invite.url}
          sealed={invite.sealed}
          plainName={invite.serverName}
          expiresAt={invite.expiresAt}
          hasBackground={invite.hasBackground}
        />
      </div>
      <p className="muted small invite-note">
        Commy servers are peer to peer: joining connects you directly to the members who are online, and nothing
        you send passes through {host}. Only join servers from people you trust.
      </p>
    </main>
  );
}
