import { chromium } from "playwright";
import { avatarSvg, gameSvg, islandSvg, serverIconSvg, svgDataUrl } from "./art.mjs";

const URL = process.env.UI_URL || "http://host.docker.internal:5174/";
// A cute cartoon cat saying hi, from KLIPY, the GIF search built into Commy.
const DEFAULT_GIF = "https://static.klipy.com/ii/c3a19a0b747a76e98651f2b9a3cca5ff/ea/4c/Zpt8i5To.gif";
const GIF_URL = process.env.GIF_URL ?? DEFAULT_GIF;
const OUT = process.env.OUT || "/work/out";
// IGDB cover image ids for the games members are playing, when known (see README).
const COVERS = process.env.COVERS ? JSON.parse(process.env.COVERS) : {};
const only = process.argv.slice(2);

const PEOPLE = [
  ["me", "commander"], ["m1", "ferris"], ["m2", "nova"], ["m3", "quill"], ["m4", "juniper"],
  ["m5", "orbit"], ["m6", "sprocket"], ["m7", "mira"], ["m8", "pixel"],
];
const avatars = Object.fromEntries(PEOPLE.map(([id], i) => [`av-${id}`, svgDataUrl(avatarSvg(i))]));
const assets = {
  avatars,
  island: svgDataUrl(islandSvg()),
  game: svgDataUrl(gameSvg()),
  icon: svgDataUrl(serverIconSvg()),
  gif: GIF_URL,
  covers: COVERS,
};

// Runs in the page: replaces the preview's sample data with a demo community.
function seed(assets) {
  const ago = mins => new Date(Date.now() - mins * 60e3).toISOString();
  window.mockSeed = () => {
    const id = () => crypto.randomUUID();
    const cat = (name, position) => ({ id: id(), name, kind: "category", topic: null, voice_users: [], position });
    const none = { id: null };
    const ch = (name, kind, parent, topic, extra = {}) => ({ id: id(), name, kind, topic: topic || null, voice_users: [], parent: parent.id, ...extra });
    const channels = [

      ch("welcome", "text", none, "Say hi and grab your roles"),
      ch("announcements", "announcement", none, "Game nights and updates"),
      ch("general", "text", none, "Anything goes"),
      ch("screenshots", "text", none, "Show off your builds"),
      ch("game-night", "text", none, "Plans for Friday"),
      ch("Lounge", "voice", none, null, { voice_users: ["commander", "ferris", "nova", "sprocket"], voice_user_ids: ["me", "m1", "m2", "m6"] }),
      ch("Game Night", "voice", none, null, { voice_users: ["orbit", "mira"], voice_user_ids: ["m5", "m7"], user_limit: 8 }),
      ch("Quiet Room", "voice", none),
    ].map((c, i) => ({ position: i, ...c }));
    const server = {
      id: id(), name: "Pixel Pioneers", owner: "me", icon: "icon@mock", channels,
      members: [
        { id: "m1", name: "ferris", presence: "online", custom_status: "building a castle", avatar: "blob:av-m1", color: "ff7a59", tag: "MOD", tag_color: "ff7a59" },
        { id: "m2", name: "nova", presence: "online", custom_status: null, activity: "Playing Minecraft", avatar: "blob:av-m2", color: "8a7dff" },
        { id: "m3", name: "quill", presence: "idle", custom_status: "brb coffee", avatar: "blob:av-m3" },
        { id: "m4", name: "juniper", presence: "online", custom_status: null, activity: "Playing Stardew Valley", avatar: "blob:av-m4", color: "2fd3a7" },
        { id: "m5", name: "orbit", presence: "dnd", custom_status: "in a match", activity: "Playing Rocket League", avatar: "blob:av-m5", color: "8a7dff" },
        { id: "m6", name: "sprocket", presence: "online", custom_status: null, activity: "Playing Trackmania", avatar: "blob:av-m6", color: "ffcf4d", tag: "RACE", tag_color: "ffcf4d" },
        { id: "m7", name: "mira", presence: "online", custom_status: null, avatar: "blob:av-m7" },
        { id: "m8", name: "pixel", presence: "offline", custom_status: null, avatar: "blob:av-m8" },
      ],
    };
    for (const member of server.members) {
      const game = member.activity?.replace(/^Playing /, "");
      if (game && assets.covers[game]) member.activity_cover = assets.covers[game];
    }
    server.roles = [
      { id: "r0", name: "Admin", color: "ff5c8a", position: 4, permissions: ["administrator"], self_assignable: false, tag: null },
      { id: "r1", name: "Moderator", color: "ff7a59", position: 3, permissions: ["manage_messages", "kick_members", "mute_members"], self_assignable: false, tag: "MOD" },
      { id: "r2", name: "Racer", color: "ffcf4d", position: 2, permissions: [], self_assignable: true, tag: "RACE" },
      { id: "r3", name: "Builder", color: "2fd3a7", position: 1, permissions: [], self_assignable: true, tag: null },
    ];
    server.game_tags = ["Trackmania", "Minecraft", "Rocket League"];
    server.rules = "# Welcome to Pixel Pioneers\n1. Be kind\n2. No spam\n3. Keep voice friendly";
    server.needs_rules = false;
    server.sounds = [];
    server.emojis = [];
    mock.servers[server.id] = server;
    mock.serverImages = { "icon@mock": assets.icon };
    mock.avatars = { ...assets.avatars, "img-island": assets.island };

    const general = channels.find(c => c.name === "general").id;
    const m = (mid, author_id, author_name, content, mins, extra = {}) =>
      ({ id: mid, channel: general, author_id, author_name, content, created_at: ago(mins), edited: false, attachments: [], ...extra });
    mock.messages[general] = [
      m("g1", "m4", "juniper", "morning everyone! who is around for game night on Friday?", 52),
      m("g2", "m1", "ferris", "count me in. I'll set up the Game Night voice channel with an 8 person limit", 50,
        { reactions: [{ emoji: "\u{1F525}", count: 3, users: ["m2", "m4", "m6"] }] }),
      m("g3", "m6", "sprocket", "finally finished the island base, look", 31,
        { attachments: [{ blob: "img-island", filename: "island-base.png", size_bytes: 482113, mime: "image/png", is_image: true }],
          reactions: [{ emoji: "\u{1F60D}", count: 5, users: ["m1", "m2", "m3", "m4", "me"] }, { emoji: "\u{1F3DD}️", count: 2, users: ["m7", "m5"] }] }),
      m("g4", "m2", "nova", "@sprocket that little house on the hill is so good. is the world file up anywhere?", 29,
        { reply_to: "g3", reply_preview: { author_id: "m6", author_name: "sprocket", excerpt: "finally finished the island base, look" } }),
      m("g5", "m6", "sprocket", "dropping it here, it goes straight from my machine to yours", 27,
        { attachments: [{ blob: "file-world", filename: "island-base-world.zip", size_bytes: 48_211_456, mime: "application/zip", is_image: false }] }),
      m("g6", "m3", "quill", "bringing a friend on Friday, sending them the invite https://commy.chat/i/K7QX2M9P", 12),
      m("g7", "me", "commander", "nice, see you all in the Lounge \u{1F3AE}", 4, { reactions: [{ emoji: "\u{1F44B}", count: 4, users: ["m1", "m2", "m4", "m6"] }] }),
    ];
    // With GIF_URL set, nova replies to the island with a real KLIPY GIF so everything still fits.
    if (assets.gif) {
      const list = mock.messages[general];
      const reply = list.find(x => x.id === "g4");
      reply.content = assets.gif;
      list.pop();
    }
    mock.dms = {
      m1: { name: "ferris", messages: [{ id: "d1", from: "m1", from_name: "ferris", content: "want to test the new soundboard before Friday?", created_at: ago(20), outgoing: false }] },
    };
    mock.reports = [];
  };
  mock.account = { signed_in: true, user: { id: "u1", name: "commander", email: "commander@example.com" }, server: "https://commy.chat",
    server_host: "commy.chat", is_default_server: true, server_from_env: false, device_linked: true, pending_code: null };
}

async function boot(page) {
  await page.addInitScript(() => { if (!crypto.randomUUID) crypto.randomUUID = () => "10000000-1000-4000-8000-100000000000".replace(/[018]/g, c => (c ^ (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (c / 4)))).toString(16)); });
  await page.goto(URL);
  await page.waitForSelector("#username-input");
  await page.evaluate(seed, assets);
  await page.fill("#username-input", "commander");
  await page.click("#username-save");
  try { await page.waitForSelector(".channel", { timeout: 8000 }); }
  catch (e) { await page.screenshot({ path: `${OUT}/fail.png` }); throw e; }
  await page.evaluate(async () => {
    state.user.avatar = "blob:av-me";
    state.user.custom_status = null;
    renderUserPanel();
    await selectServer(state.currentServer.id);
    document.querySelectorAll(".toast, #toast, .toasts").forEach(t => (t.style.display = "none"));
  });
  await page.addStyleTag({ content: ".toast, #toast, .toasts, #toasts { display: none !important; }" });
}

async function openChannel(page, name) {
  await page.evaluate(async name => {
    const c = state.currentServer.channels.find(c => c.name === name);
    await selectChannel(c);
  }, name);
}

async function joinLounge(page, speaking = ["m2"]) {
  await page.evaluate(async speaking => {
    const c = state.currentServer.channels.find(c => c.name === "Lounge");
    await invoke("set_voice_channel", { serverId: state.currentServer.id, channelId: c.id });
    state.voiceChannelId = c.id;
    state.voiceServer = { id: state.currentServer.id, name: state.currentServer.name, channelName: c.name };
    renderShareButton();
    await selectServer(state.currentServer.id);
    for (const u of speaking) setSpeaking(u, true);
  }, speaking);
}

async function shot(page, name, mouse = [1, 959]) {
  await page.mouse.move(...mouse);
  await page.waitForTimeout(500);
  await page.screenshot({ path: `${OUT}/${name}.jpg`, type: "jpeg", quality: 84 });
  console.log("saved", name);
}

const scenes = {
  async chat(page) {
    await joinLounge(page, ["m2"]);
    await openChannel(page, "general");
    // Let linked GIFs load before pinning the view to the newest message.
    await page.evaluate(async () => {
      const imgs = [...document.querySelectorAll(".image-embed img")];
      imgs.forEach(img => (img.loading = "eager"));
      await Promise.all(imgs.map(img => img.decode().catch(() => {})));
      for (let el = imgs[0]; el; el = el.parentElement) {
        if (el.scrollHeight > el.clientHeight + 1 && getComputedStyle(el).overflowY !== "visible") { el.scrollTop = el.scrollHeight; break; }
      }
    });
    await shot(page, "chat");
  },
  async voice(page) {
    await joinLounge(page, ["m1", "me"]);
    await openChannel(page, "general");
    await page.evaluate(() => openUserSettings("voice"));
    await shot(page, "voice");
  },
  async stream(page) {
    await joinLounge(page, ["m6"]);
    await page.evaluate(async game => {
      const img = new Image();
      img.src = game;
      await img.decode();
      const canvas = document.createElement("canvas");
      canvas.width = 1280;
      canvas.height = 720;
      canvas.getContext("2d").drawImage(img, 0, 0);
      const jpeg = canvas.toDataURL("image/jpeg", 0.9).split(",")[1];
      window.mockFrame = () => jpeg;
      state.currentServer.screen_shares = [{ user: "m6", name: "sprocket" }];
      renderChannels();
      renderVoicePanel();
      await selectChannel(state.currentServer.channels.find(c => c.name === "general"));
      await watchScreen("m6", "sprocket");
      clearInterval(mock.watchTimer);
      mock.watchTimer = setInterval(() => showStageFrame(`data:image/jpeg;base64,${jpeg}`, 1280, 720), 20);
    }, assets.game);
    await page.waitForTimeout(800);
    await shot(page, "stream", [900, 330]);
  },
  async call(page) {
    await joinLounge(page, ["m1", "me"]);
    await openChannel(page, "general");
    await page.evaluate(async () => {
      openConnPop();
      await new Promise(r => setTimeout(r, 1700));
      clearInterval(connTimer);
      const names = document.querySelectorAll("#conn-links .conn-name");
      if (names[1]) names[1].textContent = "nova";
      const avatars = document.querySelectorAll("#conn-links .avatar");
      if (avatars[1]) paintAvatar(avatars[1], "m2", "nova");
      for (const v of document.querySelectorAll("#conn-links .conn-value")) {
        v.textContent = v.textContent.replace(/[a-z0-9-]+\.relay\.iroh\.network\.?/, "relay.commy.chat");
      }
    });
    await shot(page, "call");
  },
  async roles(page) {
    await page.evaluate(() => openServerSettings("roles"));
    await shot(page, "roles");
  },
  async invite(page) {
    await openChannel(page, "general");
    await page.evaluate(() => {
      $("invite-link-name").textContent = "Pixel Pioneers";
      $("invite-link-host").textContent = "Invite from commy.chat, expires October 21";
      showModal("modal-invite-link");
    });
    await shot(page, "invite");
  },
  async signin(page) {
    await page.evaluate(() => {
      hideModals();
      showSignIn();
    });
    await page.click("#signin-go");
    await shot(page, "signin");
  },
};

const browser = await chromium.launch({
  args: [`--unsafely-treat-insecure-origin-as-secure=${new globalThis.URL(URL).origin}`, ...(process.env.NO_SANDBOX ? ["--no-sandbox"] : [])],
});
for (const [name, run] of Object.entries(scenes)) {
  if (only.length && !only.includes(name)) continue;
  const page = await browser.newPage({ viewport: { width: 1440, height: 960 }, deviceScaleFactor: 1.5 });
  page.on("pageerror", e => console.log("pageerror", name, e.message));
  page.on("console", m => { if (m.type() !== "log") console.log("console", name, m.text()); });
  await boot(page);
  await run(page);
  await page.close();
}
await browser.close();
