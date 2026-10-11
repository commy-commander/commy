---
title: Invite links
description: Share a server with a short link that opens Commy and joins directly.
order: 2
---

An invite link looks like `https://commy.chat/i/AB23CD45`. It points at a server invite stored on the account server that made the link.

## Making a link

In your server, open **Invite People** and choose **Copy Invite Link**. The link is copied to your clipboard. Links last 7 days, or up to 30 when made from a signed in app.

The raw `commy...` invite in the same dialog still works too. It does not expire, but it is long and only opens in the app.

## Opening a link

- **With Commy installed:** the page has an **Open in Commy** button. Commy opens and shows a Join dialog with the server name.
- **Without Commy:** download it from the same page, then paste the link into **Add a Server**.
- **In a chat:** links sent in a Commy message get a **Join** button.

## Links from other servers

Anyone can [host their own](/docs/self-hosting) Commy account server, so a link can come from a domain other than yours. Commy resolves the link on the server named in it, and when that is not your account server the Join dialog warns you first. Only join if you trust the person who sent it.

Joining never moves your account. You stay on your own account server.

## What a link reveals

An invite contains the server name and id, plus the network addresses of the member who created it, so new members can connect to them. That is true of every Commy invite, link or not. The account server stores it until the link expires.
