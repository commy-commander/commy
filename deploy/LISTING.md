# Listing a Commy server

Commy keeps a short list of community servers run by other people. The apps
show listed servers when someone picks where to sign in, and joining a
community on a listed server needs no extra step. A server that is not listed
still works: the apps ask people to confirm they trust it first.

A server can be listed for accounts (it runs the account server, so people can
sign in there) and for communities (it runs the sync host bundle from
[HOSTING.md](HOSTING.md), so communities can keep their messages there), or
both. The checks below find out which.

## The pledge

Every listed server's operator promises to:

1. **Moderate actively.** Someone reads reports and acts on them within 72
   hours, and the server has rules that forbid illegal content, harassment and
   spam.
2. **Publish an abuse contact** that a person answers within 72 hours: an email
   address or an https page, set as `COMMY_ABUSE_CONTACT` on an account server
   and `SYNC_ABUSE_CONTACT` on a sync host.
3. **Back up daily.** Keep at least 7 days of database backups somewhere other
   than the server itself, and test a restore at least once.
4. **Name an emergency admin**: a second person with access who can keep the
   server running, or shut it down cleanly, if the operator cannot.
5. **Give 3 months' notice before shutting down**, to the people who use it and
   to the listing contact, so communities can move their sync host and people
   can move their accounts.
6. **Publish their own Terms and privacy page**, written for their server, and
   link them in its settings (`COMMY_TERMS_URL` and `COMMY_PRIVACY_URL`).
7. **Keep the 18+ check on** for accounts (`ADULTS_ONLY=true`). Commy is for
   adults, and the official apps only sign in where the check is on.
8. **Stay current**: run a supported release, at most one minor release behind
   the latest, with a valid HTTPS certificate.

## Automatic checks

Commy runs these when a server applies and again every month:

| Check | What passes |
| --- | --- |
| Address | An https address on a public domain that is not on Commy's list of blocked servers |
| TLS | A certificate the apps accept, valid for at least 14 more days |
| Discovery | `/.well-known/commy-auth.json` names the server and its server key (accounts), or `/sync/about` answers (communities) |
| Version | The server reports a release no more than one minor release behind the latest |
| Contact | The abuse contact is set; an email domain accepts mail, or the contact page loads |
| Legal | The Terms and privacy pages load over https |
| 18+ | The account server reports the 18+ check on |

When the server applies, the abuse contact also gets an email with a link to
confirm, which proves a person reads it. If the contact is a web page, the
link goes to the operator's email instead.

After every check passes and the contact is confirmed, a person at Commy
reviews the application. Listing is not automatic.

## Monthly re-checks and delisting

Each listed server is checked again 30 days after its last check. If a check
fails, the operator and the abuse contact get an email naming what failed, and
the server is checked again every day. A server that still fails 7 days later
is delisted, and both get an email saying so.

A server is delisted straight away, without the 7 days, if it lands on the
list of blocked servers or breaks the pledge, for example by ignoring abuse
reports. Delisting does not stop a server working: its communities and
accounts carry on, and the apps ask people to confirm they trust it again.

A delisted server can apply again once the problem is fixed.

## How to apply

Fill in the form at `https://commy.chat/servers/apply`. It asks for:

- the server's address, for example `https://chat.example.org`
- a short name and a one-line description for the sign-in list
- the operator's email and the emergency admin's email (Commy keeps both to
  reach you about the listing and shows neither in the apps)
- links to the Terms and privacy page, if the server only hosts communities
  (an account server publishes them itself)
- your agreement to the pledge

The form runs the checks straight away and shows the result, so you can fix
anything that fails and try again.

## For the commy.chat operator

The listing runs on the official account server only, turned on with
`LISTING=on`. `LISTING_REVIEW_EMAIL` receives each application that passes its
checks, with links to approve or reject it. From the VPS:

```sh
docker compose exec auth node src/listing-cli.js list
docker compose exec auth node src/listing-cli.js show chat.example.org
docker compose exec auth node src/listing-cli.js recheck chat.example.org
docker compose exec auth node src/listing-cli.js delist chat.example.org "ignored abuse reports"
```

The signed list is served at `/api/auth/commy/server-list` with the same list
key as the server blocklist, and the apps read it from the discovery document.
