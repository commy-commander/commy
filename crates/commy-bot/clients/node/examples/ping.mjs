import { Client } from "../index.mjs";

const client = new Client({ prefix: "!" });

client.on("ready", () => {
  console.log(`Signed in as ${client.user.name} in ${client.servers.size} server(s)`);
});

client.on("memberJoin", member => {
  const welcome = member.server.channel("general");
  if (welcome) welcome.send(`Welcome, ${member}!`).catch(console.error);
});

client.command("ping", message => message.reply("pong"));

client.command("roll", (message, [sides = "6"]) => {
  const n = Math.max(2, Math.min(1000, Number.parseInt(sides, 10) || 6));
  return message.reply(`${message.author} rolled ${1 + Math.floor(Math.random() * n)} (d${n})`);
});

client.command("role", async (message, args) => {
  const name = args.join(" ");
  const role = message.server.role(name);
  if (!role) return message.reply(`No role called ${name}.`);
  await message.member.roles.add(role.id);
  await message.react("✅");
});

client.on("error", console.error);

await client.login({ data: process.env.COMMY_BOT_DATA || "commy-bot-data" });
