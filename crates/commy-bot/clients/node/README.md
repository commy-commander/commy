# commy-bot-client

Write Commy bots in JavaScript. This library talks to a running
[commy-bot](../../README.md) over its local bridge, so your code never handles
keys or networking. It has no dependencies and needs Node.js 18 or newer.

```sh
commy-bot run --bridge
node examples/ping.mjs
```

```js
import { Client } from "./index.mjs";

const client = new Client({ prefix: "!" });
client.on("ready", () => console.log(`Signed in as ${client.user.name}`));
client.command("ping", message => message.reply("pong"));
await client.login({ data: "commy-bot-data" });
```

`login` reads the token from `bridge.token` in the bot's data folder. You can
pass `{ token }` or set `COMMY_BOT_TOKEN` instead, and `{ port }` if the bridge
is not on 47620.

## Events

| Event | Arguments |
|---|---|
| `ready` | the client, once signed in; `client.servers` holds every server |
| `serverReady` | a `Server` the bot just got its copy of |
| `messageCreate`, `messageUpdate` | a `Message` |
| `messageDelete` | `{ id, channel, server }` |
| `messageReactionAdd`, `messageReactionRemove` | `{ emoji, messageId, channel, server }`, the `Member` who reacted |
| `memberJoin`, `memberLeave` | a `Member` |
| `commandInvoked` | the `Message` that used a slash command, then `{ name, options }` |
| `close`, `error`, `warn` | |

## Objects

- `Client`: `user`, `servers`, `server(nameOrId)`, `command(name, run)`, `slashCommand(name, run)`, `registerCommands(server, commands)`, `login(options)`, `destroy()`, `refresh(serverId)`.
- `Server`: `id`, `name`, `channels`, `roles`, `members`, `channel(nameOrId)`, `role(nameOrId)`, `member(id)`.
- `Channel`: `id`, `name`, `kind`, `send(textOrOptions)`.
- `Message`: `id`, `content`, `author`, `member`, `channel`, `server`, `createdAt`, `editedAt`, `replyTo`, `mentions`, `attachments`, `reactions`, `late`, `reply()`, `edit()`, `delete()`, `react(emoji)`, `unreact(emoji)`.
- `Member`: `id`, `name`, `username`, `bot`, `owner`, `roles.has()`, `roles.add()`, `roles.remove()`, `kick()`, `ban()`.

`send`, `reply` and `edit` take a string or `{ content, embeds }`. Every action
returns a promise that rejects with the bot's reason when it is not allowed.

Run the tests with `npm test`.
