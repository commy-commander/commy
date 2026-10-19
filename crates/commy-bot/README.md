# commy-bot

A Commy member that runs on its own, without a window, and keeps chat tidy.
It joins a server with an invite like anyone else, has its own identity key,
and shows a **BOT** badge next to its name. Everything it does is signed with
that key, and it only removes or bans when its roles allow it. Members' apps
check bans and role changes against its roles; message removal is not yet
checked by other members (any member's signed removal is honored today), so
keep the data folder as private as an account password.

Today it does one job: it enforces the server's **Auto Moderation** rules
(Server Settings > Auto Moderation). Auto moderation already hides messages
that break the rules from members; the bot removes them for everyone, can post
a short note in a channel you choose, and can ban someone after repeated
removals.

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

`COMMY_RELAY_URLS` picks relays the same way as the desktop app.

## What it does and does not do

- It acts on new messages and edits that break the rules: anything posted or
  edited while it is running and less than ten minutes old. It does not sweep older history, so
  changing the rules never deletes old messages on its own.
- Messages from the owner, from anyone with Manage Messages or Manage Server,
  and from exempt roles are never touched, the same as auto moderation.
- Removals and bans are signed by the bot and are permanent like any
  moderator's.
- The data folder holds the bot's private keys. Keep it private and back it up;
  a new folder is a new bot.
