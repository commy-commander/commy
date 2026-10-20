# commy-bot

A Commy member that runs on its own, without a window. It keeps chat tidy, runs
bot code you write or bring over from elsewhere, and posts webhooks from other
services into your channels.
It joins a server with an invite like anyone else, has its own identity key,
and shows a **BOT** badge next to its name. Everything it does is signed with
that key, and it only removes or bans when its roles allow it. Members' apps
check bans and role changes against its roles; message removal is not yet
checked by other members (any member's signed removal is honored today), so
keep the data folder as private as an account password.

Out of the box it enforces the server's **Auto Moderation** rules
(Server Settings > Auto Moderation). Auto moderation already hides messages
that break the rules from members; the bot removes them for everyone, can post
a short note in a channel you choose, and can ban someone after repeated
removals. On top of that:

- **Your own bot code**, in any language, connects to it over a local bridge
  (`--bridge`). A JavaScript library is in [clients/node](clients/node). See
  [Porting a bot](#porting-a-bot).
- **Incoming webhooks** let other services (build servers, feeds, monitoring)
  post into a channel (`--webhooks`). See [Webhooks](#webhooks).

## Hosting

Commy does not host bots. A bot is online only while the computer running it
is, so run it on a machine that stays on: your own PC, a home server or a small
VPS. It needs no open ports; it connects out like the desktop app.

## Set up

Build it from this repository:

```sh
cargo build --release -p commy-bot
```

The binary is `target/release/commy-bot` (`commy-bot.exe` on Windows).

1. **Create the bot.** This makes a data folder holding its keys.

   ```sh
   commy-bot init Tidy
   ```

2. **Invite it.** In the app, open the server, choose **Invite People** and press
   **Copy Invite Code** (the code starts with `commy`). Then:

   ```sh
   commy-bot join "commy..."
   ```

3. **Start it.**

   ```sh
   commy-bot run --log-channel mod-log --ban-after 3
   ```

   The first time, it waits until the server owner's app is online, because it
   only trusts a copy of the server signed by the owner. After that it
   remembers the server.

4. **Give it a role.** In Server Settings > Roles, create a role with
   **Manage Messages** (and **Ban Members** if you use `--ban-after`), and assign
   it to the bot. Without Manage Messages it joins but does nothing.

Stop it with Ctrl+C. `commy-bot servers` shows each server and what the bot may
do there.

## Options

| Option | What it does |
|---|---|
| `--data FOLDER` | Where the bot keeps its keys and servers. Defaults to `$COMMY_BOT_DATA`, then `./commy-bot-data`. One bot per folder. |
| `--log-channel NAME` | Posts "Removed a message from NAME: blocked word" in that text channel. The bot needs permission to post there. |
| `--ban-after COUNT` | Bans a member after COUNT removed messages within an hour. Needs Ban Members. Moderators and the owner are never banned. |
| `--bridge [PORT]` | Lets bot code connect on 127.0.0.1 (port 47620 unless given). Only programs on the same machine that know the token can connect. |
| `--webhooks [ADDRESS]` | Accepts webhook posts on 127.0.0.1:47621, or on the port or address given. |

`COMMY_RELAY_URLS` picks relays the same way as the desktop app.

## What it does and does not do

- It acts on messages and edits that break the rules and were written after it
  joined the server, up to a day old. That includes ones it only learns about
  when it starts again or reconnects and catches up with other members. It does
  not sweep older history, so changing the rules never deletes old messages on
  its own.
- Messages from the owner, from anyone with Manage Messages or Manage Server,
  and from exempt roles are never touched, the same as auto moderation.
- Removals and bans are signed by the bot and are permanent like any
  moderator's.
- The data folder holds the bot's private keys. Keep it private and back it up;
  a new folder is a new bot.

## Porting a bot

Most bots are a script that signs in, listens for events and calls actions.
`commy-bot` does the signing in, the networking and the signatures; your code
talks to it over the bridge, a newline-delimited JSON connection on 127.0.0.1.

1. Start the bot with the bridge on:

   ```sh
   commy-bot run --bridge
   ```

   The first run writes a token to `bridge.token` in the data folder.
   `commy-bot token` prints it and `commy-bot token --reset` replaces it.

2. Connect your code. In JavaScript, with the library in
   [clients/node](clients/node):

   ```js
   import { Client } from "./clients/node/index.mjs";

   const client = new Client({ prefix: "!" });
   client.on("messageCreate", message => {
     if (message.content === "hello") message.reply(`hi ${message.author}`);
   });
   client.command("ping", message => message.reply("pong"));
   await client.login({ data: "commy-bot-data" });
   ```

   Other languages speak the protocol directly; see [The bridge protocol](#the-bridge-protocol).
   Rust code can skip the bridge: implement `commy_bot::Handler` (or pass a
   closure) and add it with `Bot::with`.

3. Give the bot the roles its actions need: Send Messages to post, Manage
   Messages to delete other people's messages, Kick Members, Ban Members,
   Manage Roles. Every action is signed with the bot's key, and members' apps
   check kicks, bans and role changes against the bot's roles (message removal
   is not checked by other members yet, as noted above). The bot checks its own
   roles before acting, so a missing role fails with a reason such as "the bot
   needs a role with Ban Members" instead of being sent.

### What carries over

| Familiar idea | In Commy |
|---|---|
| Prefix commands (`!ping`) | Work as they are. `client.command(name, run)` routes them. |
| Message, edit, delete and reaction events | `messageCreate`, `messageUpdate`, `messageDelete`, `messageReactionAdd`, `messageReactionRemove` |
| Member joins and leaves | `memberJoin`, `memberLeave` |
| Reply, react, edit or delete a message | `message.reply`, `react`, `unreact`, `edit`, `delete` (edits only on the bot's own messages) |
| Kick, ban, add or remove a role | `member.kick()`, `member.ban()`, `member.roles.add(role)`, `member.roles.remove(role)` |
| Mentions written as `<@id>`, `<@!id>`, `<#id>`, `<@&id>` | Turned into `@name`, `#channel` and `@role` before sending. `${member}` and `${channel}` produce these. |
| Timestamps written as `<t:1700000000>` | Turned into a UTC date and time. |
| Custom emoji written as `<:name:id>` | This server's emoji of that name, or `:name:`. |
| Rich cards ("embeds" with title, description, fields, url, image, footer) | Flattened into plain lines under the message text. Commy messages are plain text with links; image links show as click-to-load images. Objects with `toJSON()` are accepted. |
| Servers, channels and roles by id | Ids are Commy ids. Channels and roles can also be given by name. |

Messages the bot learns about late (written more than two minutes ago, for
example while it was offline) arrive with `late: true`. The JavaScript library
skips them unless you pass `includeLate: true`, so a restarted bot does not
answer old commands.

### Not supported yet

- Slash commands with an in-app picker, buttons, menus and pop-up forms.
- Replies that only one person can see, direct messages to or from bots,
  threads, and bots in voice channels.
- Card layouts for rich messages (they are flattened to text).
- Reading history older than one day, or from before the bot joined.
- Hosting. Commy does not run bots for you; a bot is online while its machine is.
- Plugins that run inside the Commy app.

### The bridge protocol

One JSON object per line, UTF-8, at most 64 KiB per line. The first line from
your code must be `{"op": "hello", "token": "..."}`. The bot answers with
`{"type": "hello", "version": 1, "me": {...}, "servers": [...]}`, or
`{"type": "denied", "error": "..."}` and closes.

After that, the bot sends events:

| `type` | Fields |
|---|---|
| `server_ready` | `server` (the bot just got its copy of a server) |
| `message_create`, `message_update` | `server`, `message` |
| `message_delete` | `server`, `channel`, `message` (id) |
| `reaction_add`, `reaction_remove` | `server`, `channel`, `message` (id), `user`, `emoji` |
| `member_join` | `server`, `member` |
| `member_leave` | `server`, `user` (id) |
| `lagged` | `missed`: events dropped because your code read too slowly |

A server is `{id, name, ready, owner, channels: [{id, name, kind}], roles: [{id, name, position}], members: [...]}`.
A member is `{id, name, username, bot, owner, roles: [role ids]}`.
A message is `{id, server, channel, channel_name, author, content, created_at, edited_at, reply_to, attachments, reactions, mentions, late}`.

Your code sends actions. Each may carry a `nonce` of your choice, which comes
back in its `{"type": "result", "nonce": ..., "ok": true}` or
`{"type": "result", "nonce": ..., "ok": false, "error": "..."}` answer.

| `op` | Fields | Answer |
|---|---|---|
| `send` | `server`, `channel`, `content` and/or `embeds`, optional `reply_to` | `message` |
| `edit` | `server`, `message`, `content` and/or `embeds` | |
| `delete` | `server`, `message` | |
| `react`, `unreact` | `server`, `message`, `emoji` | |
| `kick`, `ban` | `server`, `user` | |
| `add_role`, `remove_role` | `server`, `user`, `role` | |
| `server` | `server` | `server` with its members |

`server` may be an id or a server name, `channel` an id, `#name` or `<#id>`,
`user` an id, `@name` or `<@id>`, and `role` an id, a name or `<@&id>`.

## Webhooks

A webhook is a secret URL that posts into one channel as the bot.

```sh
commy-bot webhook add "Rust Hangout" builds "Build server"
commy-bot run --webhooks
```

`webhook add` prints the URL. Send it a POST with a JSON body:

```sh
curl -X POST -H "Content-Type: application/json" \
  -d '{"content": "Build 512 passed", "embeds": [{"title": "main", "url": "https://example.com/builds/512"}]}' \
  http://127.0.0.1:47621/webhooks/ID/SECRET
```

`content` and `embeds` work as in [What carries over](#what-carries-over); other
fields are ignored. Answers: 204 posted, 400 not JSON, 404 unknown or removed
webhook, 413 body over 32 KiB, 422 empty or blocked by auto moderation,
429 more than 30 posts a minute, 403 the bot lacks permission to post there,
410 the channel or server is gone, 503 the bot has no copy of the server yet.

`commy-bot webhook list` shows them, and `commy-bot webhook remove ID` stops one
working at once. Webhooks listen on 127.0.0.1 only unless you pass an address.
For a service on the internet to reach one, put the port behind a reverse proxy
with HTTPS on the machine that runs the bot, and treat each URL like a password.
