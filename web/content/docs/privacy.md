---
title: Privacy
description: What an account server stores, what it never sees, and who else is involved.
order: 5
---

Commy messages, voice, video, screen shares and files are peer to peer. They never pass through commy.chat or any other account server.

## What the account server stores

- **Accounts:** email, display name, avatar URL and a password hash (scrypt).
- **Google sign in:** the Google account id. Google tokens are encrypted at rest and never used after sign in.
- **Sessions:** a token and its expiry. IP addresses and browser details are not stored.
- **Linked devices:** each device's iroh node id, its name, and when it was linked.
- **Invite links:** the invite, server name, creator account if signed in, and expiry date.
- **Short lived records:** email verification, desktop sign in codes, and a keyed hash of the network a sign in request came from (deleted after 15 minutes, never the raw IP).

IP addresses are used in memory for rate limiting only. The web server keeps no access logs, and the auth server never logs emails, links or tokens.

## What it never sees

Messages, DMs, voice, video, screen shares, files, which servers you are in, your friends, or who talks to whom.

## Who else is involved

- **The people you connect to** see your IP address, as in any direct call or game lobby.
- **iroh relays** see IP addresses and node ids when they help peers connect, but only ever carry encrypted traffic.
- **Google** sees your sign in if you use Google, and **your email provider** delivers verification and sign in emails.
- **Cloudflare Turnstile** runs a bot check on the sign up and sign in pages, only with your consent (see [Cookies](#cookies)).

## Cookies

commy.chat has no analytics, ads or tracking of any kind. Fonts and images are served from the site itself.

Until you choose in the cookie banner, the site writes nothing to your device and loads nothing optional.

**Strictly necessary, set only when you act:**

- `commy_consent`: remembers whether you accepted or rejected, for 180 days. Set when you click a button in the banner.
- Account cookies: the session cookie, plus short-lived cookies that protect a sign in while it is in progress. The account server sets them only when you sign in, and removes the session when you sign out.

**Optional, only after you accept:**

- **Cloudflare Turnstile**, the bot check on the sign in and sign up forms. It loads a script from Cloudflare that may store data on your device. If you reject, the forms ask before loading it, just for that form.

Change your choice any time with **Cookie settings** at the bottom of every page.

## Your own server

Everything above is true of the open source code you can [host yourself](/docs/self-hosting), where you are the only operator.
