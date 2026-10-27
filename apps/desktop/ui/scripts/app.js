"use strict";

const IS_TAURI = Boolean(window.__TAURI__);
const invoke = IS_TAURI ? window.__TAURI__.core.invoke : mockInvoke;
const discoverState = { open: false, q: "", category: null, cursor: null, entries: [], busy: false, request: 0, host: "" };
const discoveryState = { listing: null, dirty: false };

window.addEventListener("error", e => {
  try { toast(`UI error: ${e.message}`); } catch (_) {}
});
window.addEventListener("unhandledrejection", e => {
  try { toast(`UI error: ${e.reason}`); } catch (_) {}
});

const state = {
  user: null,
  servers: [],
  currentServer: null,
  currentChannel: null,
  voice: { noise_cancellation: true, echo_cancellation: true, auto_level: true, muted: false, deafened: false, output_volume: 1, input_gain: 1, stream_volume: 1, afk_timeout_minutes: 10 },
  voiceChannelId: null,
  voiceServer: null,
  shareSource: null,
  watchingName: null,
  sharing: false,
  watchingUser: null,
  view: "server",
  currentDm: null,
  micTesting: false,
  people: new Map(),
  speaking: new Set(),
  dmThreads: [],
  showDmProfile: true,
  historyAnchor: null,
  previousChannel: null,
};

const mock = {
  user: null,
  servers: {},
  messages: {},
  dms: {},
  voice: { noise_cancellation: true, echo_cancellation: true, auto_level: true, muted: false, deafened: false, output_volume: 1, input_gain: 1, stream_volume: 1, afk_timeout_minutes: 10 },
};

function mockSearchMatcher({ terms, from, mentions }, named) {
  return (authorId, content) => {
    if (from && authorId !== from) return false;
    if (mentions && !(named && mentionsName(content, [named.name]))) return false;
    const text = content.toLowerCase();
    return terms.every(t => text.includes(t.toLowerCase()));
  };
}

function mockPage(hits, offset) {
  const total = hits.length;
  const at = total ? Math.min(Math.max(0, offset || 0), Math.floor((total - 1) / 50) * 50) : 0;
  return { total, offset: at, hits: hits.slice(at, at + 50) };
}

function mockSearch({ serverId, terms, from, mentions, channelId, offset }) {
  const server = mock.servers[serverId];
  const hidden = mockChatRestricted(server, "read_history");
  const matches = mockSearchMatcher({ terms, from, mentions }, server.members.find(m => m.id === mentions));
  const hits = server.channels
    .filter(c => holdsMessages(c) && (!channelId || c.id === channelId))
    .flatMap(c => (hidden ? historyWindow(mock.messages[c.id] || [], (mock.channelOpens || {})[c.id]) : mock.messages[c.id] || []).map(message => ({ channel: c.id, channel_name: c.name, message })))
    .filter(({ message }) => matches(message.author_id, message.content))
    .sort((a, b) => Date.parse(b.message.created_at) - Date.parse(a.message.created_at));
  return mockPage(hits, offset);
}

function mockSearchDms({ terms, from, mentions, userId, offset }) {
  const contact = mock.dms[mentions];
  const named = contact ? { name: contact.name } : mentions === "me" && mock.user ? { name: mock.user.username } : null;
  const matches = mockSearchMatcher({ terms, from, mentions }, named);
  const dms = Object.entries(mock.dms)
    .filter(([user]) => !userId || user === userId)
    .flatMap(([user, d]) => d.messages.filter(m => matches(m.from, m.content)).map(message => ({ kind: "dm", user, name: d.name, message })));
  const groups = userId ? [] : Object.values(mock.servers).filter(s => s.group).flatMap(s => s.channels.filter(holdsMessages).flatMap(c =>
    (mock.messages[c.id] || []).filter(m => matches(m.author_id, m.content)).map(message => ({ kind: "group", server: s.id, name: s.name, channel: c.id, message }))));
  const hits = [...dms, ...groups].sort((a, b) => Date.parse(b.message.created_at) - Date.parse(a.message.created_at));
  return mockPage(hits, offset);
}

function mockReadView(channelIds) {
  const channels = {};
  for (const id of channelIds) {
    const mark = (mock.readMarks || {})[id];
    if (!mark) continue;
    const server = Object.values(mock.servers).find(s => s.channels.some(c => c.id === id));
    channels[id] = { mark, server: server ? server.id : null, unread: unreadAfter(mock.messages[id] || [], mark, "me") };
  }
  return { channels };
}

function mockAuditLog() {
  const ago = minutes => Date.now() - minutes * 60e3;
  return [
    { at: ago(4), by: "m2", by_name: "nova", kind: "message_delete", target: "m3", label: "sleepy", detail: "welcome" },
    { at: ago(26), by: "me", by_name: mock.user ? mock.user.username : "you", kind: "role_assign", target: "m2", label: "nova", detail: "Moderator" },
    { at: ago(70), by: "m2", by_name: "nova", kind: "voice_moderate", target: "m3", label: "sleepy", detail: "muted" },
    { at: ago(95), by: "m2", by_name: "nova", kind: "voice_move", target: "m1", label: "ferris", detail: "General" },
    { at: ago(180), by: "me", by_name: mock.user ? mock.user.username : "you", kind: "channel_rename", target: null, label: "chat", detail: "general" },
    { at: ago(600), by: "m1", by_name: "ferris", kind: "emoji_add", target: null, label: "ferris_wave", detail: null },
    { at: ago(900), by: "me", by_name: mock.user ? mock.user.username : "you", kind: "member_nickname", target: "m3", label: "sleepy", detail: "Sleepy Sam" },
    { at: ago(1500), by: "me", by_name: mock.user ? mock.user.username : "you", kind: "member_ban", target: "m9", label: "spam_bot", detail: null },
    { at: ago(3000), by: "me", by_name: mock.user ? mock.user.username : "you", kind: "channel_create", target: null, label: "news", detail: "announcement" },
    { at: ago(4000), by: "me", by_name: mock.user ? mock.user.username : "you", kind: "auto_mod", target: null, label: null, detail: null },
  ];
}

function mockInvites() {
  if (!mock.invites) {
    const me = mock.user ? mock.user.username : "you";
    const at = minutes => Date.now() + minutes * 60e3;
    mock.invites = [
      { origin: "https://commy.chat", host: "commy.chat", code: "pixel-pioneers", custom: true, creator: "me", creator_name: me, created_at: at(-2900), expires_at: null, can_revoke: true },
      { origin: "https://commy.chat", host: "commy.chat", code: "KQ7M3N4P", custom: false, creator: "me", creator_name: me, created_at: at(-60), expires_at: at(6 * 1440 + 23 * 60), can_revoke: true },
      { origin: "https://commy.chat", host: "commy.chat", code: "T4WZ8RHD", custom: false, creator: "m2", creator_name: "nova", created_at: at(-15), expires_at: at(45), can_revoke: true },
    ];
  }
  mock.invites = mock.invites.filter(i => i.expires_at == null || i.expires_at > Date.now());
  return mock.invites;
}

function mockCrabImage() {
  const c = document.createElement("canvas");
  c.width = c.height = 64;
  const g = c.getContext("2d");
  g.fillStyle = "#ff4d4d";
  g.beginPath();
  g.ellipse(32, 36, 24, 18, 0, 0, Math.PI * 2);
  g.fill();
  for (const x of [20, 44]) {
    g.beginPath();
    g.arc(x, 24, 7, 0, Math.PI * 2);
    g.fill();
  }
  g.fillStyle = "#0b0d17";
  for (const x of [20, 44]) {
    g.beginPath();
    g.arc(x, 24, 3, 0, Math.PI * 2);
    g.fill();
  }
  return c.toDataURL("image/png");
}

function mockBannerImage() {
  const c = document.createElement("canvas");
  c.width = 960;
  c.height = 320;
  const g = c.getContext("2d");
  const grad = g.createLinearGradient(0, 0, 960, 320);
  grad.addColorStop(0, "#2a1d5c");
  grad.addColorStop(0.6, "#8a3a7a");
  grad.addColorStop(1, "#ff8a5b");
  g.fillStyle = grad;
  g.fillRect(0, 0, 960, 320);
  g.fillStyle = "rgba(255,255,255,.75)";
  for (let i = 0; i < 70; i++) {
    const x = (i * 137) % 960, y = (i * 97) % 320, r = 0.6 + (i % 4) * 0.5;
    g.beginPath();
    g.arc(x, y, r, 0, Math.PI * 2);
    g.fill();
  }
  return c.toDataURL("image/jpeg", 0.85);
}

function mockVoiceAttachment(secs) {
  const blob = `voice-${Date.now()}@mock`;
  (mock.voiceClips ||= {})[blob] = secs;
  return { filename: VOICE_MESSAGE_FILENAME, size_bytes: 400 + secs * 4000, is_image: false, mime: VOICE_MESSAGE_MIME, blob, provider: null };
}

function mockOgg(secs) {
  const page = (granule, body, flags) => {
    const out = new Uint8Array(28 + body.length);
    out.set([0x4f, 0x67, 0x67, 0x53, 0, flags]);
    new DataView(out.buffer).setUint32(6, granule, true);
    out[26] = 1;
    out[27] = body.length;
    out.set(body, 28);
    return out;
  };
  const head = new Uint8Array([79, 112, 117, 115, 72, 101, 97, 100, 1, 1, 56, 1, 128, 187, 0, 0, 0, 0, 0]);
  const pages = [page(0, head, 2), page(312 + secs * 48000, new Uint8Array(4), 4)];
  const out = new Uint8Array(pages[0].length + pages[1].length);
  out.set(pages[0]);
  out.set(pages[1], pages[0].length);
  return out.buffer;
}

const MOCK_OPPONENT = "m1";
let mockActivitySeq = 0;

function mockActivityOpponent(server, channel, input) {
  const activity = channel.activity;
  if (activity.kind !== "tic_tac_toe" || !(channel.voice_user_ids || []).includes(MOCK_OPPONENT)) return;
  const send = (payload, delay) => setTimeout(() => onActivityInput({ server: server.id, channel: channel.id, seq: ++mockActivitySeq, by: MOCK_OPPONENT, ...payload }), delay);
  const run = activityUi.run;
  if (input === "join" && activity.starter === MOCK_OPPONENT) {
    const board = [1, 0, 0, 0, 2, 0, 0, 0, 1];
    send({ input: { sync: { participants: ["me", "m2"], players: [MOCK_OPPONENT, "me"], board, playing: false, position: 0, at: 0 } } }, 300);
    return;
  }
  if (activity.starter === "me" && run && run.players.length < 2 && !isParticipant(run, MOCK_OPPONENT)) send({ input: "join" }, 800);
  if (input && input.move) {
    const free = run ? run.board.map((c, i) => (c === 0 ? i : -1)).filter(i => i >= 0) : [];
    if (free.length && !gameOver(run.board)) send({ input: { move: { cell: free[Math.floor(Math.random() * free.length)] } } }, 700);
  }
  if (input === "rematch") send({ input: { move: { cell: 4 } } }, 900);
}

function mockSeed() {
  const ch = (name, kind, topic) => ({
    id: crypto.randomUUID(), name, kind, topic: topic || null, voice_users: [],
  });
  const server = {
    id: crypto.randomUUID(),
    name: "Rust Hangout",
    owner: "owner",
    channels: [
      ch("welcome", "text", "Start here"),
      ch("general", "text", "Anything goes"),
      ch(":ferris:-rust-help", "text", "Borrow checker support group"),
      ch("news", "announcement", "Patch notes and server news"),
      ch("General", "voice"),
      ch("mod-lounge", "text", "For the moderators"),
      ch("Lounge", "voice"),
      ch("after-dark", "text", "Grown-up talk, after hours"),
      ch("Town Hall", "stage"),
    ],
    members: [
      { id: "m1", name: "ferris", presence: "online", custom_status: "heads down", about: "Crab by day, raid lead by night.", profile_policy: "friends",
        activity: "Playing Hollow Knight", activity_since: Math.floor(Date.now() / 1000) - 4332 },
      { id: "m2", name: "nova", pronouns: "they / them", presence: "idle", custom_status: null, roles: ["r2"],
        activity: "Playing Portal 2", activity_cover: "co1rs4", activity_since: Math.floor(Date.now() / 1000) - 983 },
      { id: "m3", name: "sleepy", presence: "offline", custom_status: null },
      { id: "m5", name: "quill", presence: "offline", custom_status: null, about: "Only friends get to read this.", pronouns: "she / her", profile_policy: "friends" },
      { id: "m4", name: "Tidy", presence: "online", custom_status: "Keeping chat tidy", bot: true, roles: ["r2"] },
      { id: "m10", name: "Quotebook", presence: "offline", custom_status: null, bot: true },
      { id: "m6", name: "ash", presence: "online", custom_status: "hosting tonight", roles: ["r2"] },
      { id: "m7", name: "pip", presence: "online", custom_status: null },
      { id: "m8", name: "lumen", presence: "idle", custom_status: null },
      { id: "m9", name: "dot", presence: "online", custom_status: "lurking" },
    ],
  };
  server.roles = [
    { id: "r1", name: "Raider", color: "9b7bff", position: 2, permissions: [], self_assignable: true, tag: "RAID", mentionable: true },
    { id: "r2", name: "Moderator", color: "3ba55c", position: 3, permissions: ["manage_messages", "mention_everyone"], self_assignable: false, hoist: true, tag: null, mentionable: false },
    { id: "r3", name: "Event Pings", color: "4d9bff", position: 1, permissions: [], self_assignable: false, hoist: false, tag: null, mentionable: true },
  ];
  server.onboarding = {
    questions: [
      { title: "What brings you here?", description: "Pick everything that fits. The channels for each open up for you.", multiple: true, answers: [
        { label: "Raiding and game nights", roles: ["r1"] },
        { label: "Rust help and code review", roles: [] },
        { label: "Just hanging out", roles: [] },
      ] },
      { title: "How often do you want pings?", description: "", multiple: false, answers: [
        { label: "Every event and announcement", roles: ["r3"] },
        { label: "Only when someone mentions me", roles: [] },
      ] },
    ],
  };
  server.game_tags = ["Rust", "Valorant"];
  server.emojis = [{ name: "ferris", image: "ferris@mock" }, { name: "ferris_wave", image: "ferris@mock" }];
  server.stickers = [{ name: "ferris_hype", image: "ferris@mock" }, { name: "party", image: "party@mock" }];
  server.emoticons = [{ name: "borrow_check", text: "(ง •̀_•́)ง borrow!" }, { name: "ship_it", text: "ᕦ(ò_óˇ)ᕤ ship it" }];
  mock.expressions = { ...(mock.expressions || {}), "ferris@mock": mockCrabImage(), "party@mock": mockEmojiFace(48, "M40 84 Q64 108 88 84", 8), "pixel@mock": mockEmojiFace(280, "M44 88 L84 88", 10) };
  const club = {
    id: crypto.randomUUID(), name: "Pixel Club", owner: "m2", channels: [ch("lobby", "text", null)], members: [{ id: "m2", name: "nova", presence: "idle", custom_status: null }],
    emojis: [{ name: "pixel_heart", image: "pixel@mock" }], stickers: [{ name: "pixel_wave", image: "pixel@mock" }], emoticons: [{ name: "pixel_shrug", text: "¯\\_(ツ)_/¯ pixels" }],
    emojis_shareable: true, stickers_shareable: true, emoticons_shareable: true,
  };
  mock.servers[club.id] = club;
  const squad = {
    id: crypto.randomUUID(), group: true, name: "ferris, nova", owner: "me",
    channels: [ch("chat", "text", null), ch("call", "voice", null)],
    members: [{ id: "m1", name: "ferris", presence: "online", custom_status: "heads down" }, { id: "m2", name: "nova", presence: "idle", custom_status: null }],
  };
  mock.servers[squad.id] = squad;
  mock.messages[squad.channels[0].id] = [
    { id: "g1", channel: squad.channels[0].id, author_id: "m2", author_name: "nova", content: "raid night friday?", created_at: new Date(Date.now() - 7200e3).toISOString(), edited: false, attachments: [] },
    { id: "g2", channel: squad.channels[0].id, author_id: "m1", author_name: "ferris", content: "in. I'll bring the snacks :ferris_wave:", created_at: new Date(Date.now() - 5400e3).toISOString(), edited: false, attachments: [] },
  ];
  mock.serverImages = { ...(mock.serverImages || {}), "welcome-banner@mock": mockBannerImage() };
  server.channels[0].banner = "welcome-banner@mock";
  server.channels[1].color = "9b7bff";
  server.channels[5].view_roles = ["r2"];
  server.channels[6].sensitive = true;
  mock.messages[server.channels[6].id] = [
    { id: "ad1", channel: server.channels[6].id, author_id: "m2", author_name: "nova",
      content: "Friday's game night runs late, bring your own drinks.", created_at: new Date(Date.now() - 2400e3).toISOString(), edited: false, attachments: [] },
    { id: "ad2", channel: server.channels[6].id, author_id: "m1", author_name: "ferris",
      content: "Noted. Keep the welcome channel tidy, this one is for the grown-ups.", created_at: new Date(Date.now() - 2100e3).toISOString(), edited: false, attachments: [] },
  ];
  server.rules = "# Welcome\n1. Be kind to each other\n2. **No spam** or self promotion\n3. Keep it on topic, see https://commy.chat";
  server.needs_rules = true;
  const general = server.channels.find(c => c.kind === "voice");
  general.voice_users = ["ferris"];
  general.voice_user_ids = ["m1"];
  general.voice_since = [Math.floor(Date.now() / 1000) - 754];
  general.voice_muted = [true];
  general.voice_deafened = [false];
  general.activity = { id: "mock-tic-tac-toe", kind: "tic_tac_toe", starter: "m1", started_at: Date.now() - 120e3 };
  server.members[0].voice_channel = general.id;
  server.members[0].voice_since = general.voice_since[0];
  server.members[0].voice_muted = true;
  server.members[0].priority_speaker = true;
  general.voice_status = "Ranked grind";
  const hall = server.channels.find(c => c.kind === "stage");
  const seated = [["m2", "nova", 1810], ["m6", "ash", 1790], ["m7", "pip", 640], ["m8", "lumen", 420], ["m9", "dot", 95]];
  hall.voice_users = seated.map(([, name]) => name);
  hall.voice_user_ids = seated.map(([id]) => id);
  hall.voice_since = seated.map(([, , ago]) => Math.floor(Date.now() / 1000) - ago);
  hall.voice_muted = seated.map(() => false);
  hall.voice_deafened = seated.map(() => false);
  hall.speakers = ["m2", "m6"];
  hall.hands = ["m7"];
  for (const [id, , ago] of seated) {
    const member = server.members.find(m => m.id === id);
    member.voice_channel = hall.id;
    member.voice_since = Math.floor(Date.now() / 1000) - ago;
  }
  server.members[0].tag = "RAID";
  server.members[0].tag_color = "9b7bff";
  server.members[0].color = "9b7bff";
  mock.servers[server.id] = server;
  mock.directory = [
    { code: "KQ7M3N4P", key: "q".repeat(43), name: "Rust Hangout", category: "tech", members: 250, description: "Borrow checker support group, weekly code reviews and a friendly place to ask the questions you were afraid to ask. Beginners welcome, lifetimes optional." },
    { code: "pixel-club", key: "Zx9-_".repeat(4) + "aA", name: "Pixel Club", category: "art", members: 100, description: "Pixel art, palettes and tiny animations. Share your work in progress and get kind, concrete feedback." },
    { code: "SYNTH2V4W", key: "s".repeat(43), name: "Synth Lounge", category: "music", members: 500, description: "Modular jams every Friday night, patch notes, gear talk and the occasional live set over voice." },
    { code: "RAIDN8GHT", key: "r".repeat(43), name: "Raid Night", category: "gaming", members: 1000, description: "Co-op raids, weekend tournaments and a soundboard that is honestly out of control." },
    { code: "STARGAZ3R", key: "t".repeat(43), name: "Stargazers", category: "science", members: 50, description: "Backyard astronomy, telescope builds and clear-sky alerts for the northern hemisphere." },
    { code: "LERNLAT1N", key: "l".repeat(43), name: "Lingua Franca", category: "languages", members: 25, description: "Practice Spanish, Portuguese and Italian with patient speakers. Voice tables every evening." },
    { code: "STUDYHALL", key: "u".repeat(43), name: "Study Hall", category: "education", members: 100, description: "Quiet focus rooms, exam countdowns and shared notes for university students." },
    { code: "SUNDAYRUN", key: "n".repeat(43), name: "Sunday Runners", category: "sports", members: 25, description: "Long runs, race day plans and far too many photos of shoes." },
    { code: "COZYCRN3R", key: "c".repeat(43), name: "Cozy Corner", category: "friends", members: 10, description: "" },
    { code: "BOARDGAM3", key: "b".repeat(43), name: "Board Game Night", category: "gaming", members: 50, description: "Tabletop over video, from quick card games to six-hour campaigns. New players always get a seat." },
    { code: "FILMCLUB2", key: "f".repeat(43), name: "Midnight Film Club", category: "other", members: 25, description: "One film a week, watched together with the soundtrack on voice, then argued about for days." },
    { code: "MAKERSPCE", key: "m".repeat(43), name: "Makerspace", category: "tech", members: 10, description: "3D printing, soldering irons and home automation that mostly works." },
  ];
  mock.dms.m2 = {
    name: "nova",
    kind: "request",
    messages: [
      { id: "d3", from: "m2", from_name: "nova", content: "hey, saw you in Rust Hangout. want to pair on the voice code?",
        created_at: new Date(Date.now() - 300e3).toISOString(), outgoing: false },
    ],
  };
  mock.reports = [{ id: "r1", server: server.id, channel: null, message: null, reporter: "m1", reporter_name: "ferris", reported: "m2", reported_name: "nova", reason: "spam", note: "Keeps posting invite links in every channel.", excerpt: "join my server https://example.com/invite", created_at: new Date().toISOString() }];
  const nightNotes = ["build is green", "flaky voice test again", "shipped the overlay fix", "waiting on review"];
  mock.dms.m3 = {
    name: "sleepy",
    messages: Array.from({ length: 64 }, (_, i) => ({
      id: `dl${i}`, from: i % 3 === 2 ? "me" : "m3", from_name: i % 3 === 2 ? "You" : "sleepy",
      content: `night log ${i + 1}: ${nightNotes[i % nightNotes.length]}`,
      created_at: new Date(Date.now() - 3 * 86400e3 - (64 - i) * 5400e3).toISOString(), outgoing: i % 3 === 2,
    })),
  };
  mock.dms.m1 = {
    name: "ferris",
    messages: [
      { id: "d1", from: "m1", from_name: "ferris", content: "hey, did the voice fix land?",
        created_at: new Date(Date.now() - 26 * 3600e3).toISOString(), outgoing: false },
      { id: "d0", from: "me", from_name: "You", content: "yes, it is in **0.1.2**. try the new mic test",
        created_at: new Date(Date.now() - 25 * 3600e3).toISOString(), outgoing: true, edited: true,
        reply_to: "d1", reply_preview: { author_id: "m1", author_name: "ferris", excerpt: "hey, did the voice fix land?" },
        reactions: [{ emoji: "🎉", count: 1, users: ["m1"] }] },
      { id: "d4", from: "m1", from_name: "ferris", content: "",
        created_at: new Date(Date.now() - 900e3).toISOString(), outgoing: false,
        attachments: [{ filename: "voice-fix-notes.txt", size_bytes: 2048, is_image: false, mime: "text/plain", blob: "mocknotes", provider: null }] },
      { id: "d2", from: "m1", from_name: "ferris", content: "screen share looked great btw",
        created_at: new Date(Date.now() - 600e3).toISOString(), outgoing: false },
    ],
  };
  const c0 = server.channels[0].id;
  mock.messages[c0] = [
    { id: "x1", channel: c0, author_id: "m1", author_name: "ferris",
      content: "Welcome to Commy, a private P2P chat app.", created_at: new Date(Date.now() - 3600e3).toISOString(), edited: false, attachments: [] },
    { id: "x2", channel: c0, author_id: "m2", author_name: "nova",
      content: "No central servers, no paywall. Everything peer to peer.", created_at: new Date(Date.now() - 1800e3).toISOString(), edited: false, attachments: [] },
    { id: "x2b", channel: c0, author_id: "m1", author_name: "ferris",
      content: "## Patch notes\n**Bold**, *italic*, __underline__, ~~struck~~, `inline code`, ||a spoiler|| and a [masked link](https://commy.chat).\n> Quoted from the changelog\n- noise gate tuned\n- overlay fixed\n```rust\nfn main() {\n    println!(\"hello\"); // 42\n}\n```\nShipped <t:1760900000:R>.",
      created_at: new Date(Date.now() - 1500e3).toISOString(), edited: true, attachments: [] },
    { id: "x3", channel: c0, author_id: "me", author_name: mock.user ? mock.user.username : "you",
      content: "Posted while everyone was away, so it waits on this device.", created_at: new Date(Date.now() - 120e3).toISOString(), edited: false, attachments: [], pending: true },
    { id: "x4", channel: c0, author_id: "m2", author_name: "nova",
      content: "https://commy.chat/i/kq7m3n4p#" + "q".repeat(43), created_at: new Date(Date.now() - 60e3).toISOString(), edited: false, attachments: [] },
    { id: "x5", channel: c0, author_id: "m3", author_name: "sleepy",
      content: "free gifts for everyone who clicks", created_at: new Date(Date.now() - 30e3).toISOString(), edited: false, attachments: [], held: "blocked word" },
    { id: "x6", channel: c0, author_id: "m4", author_name: "Tidy",
      content: "Removed a message from sleepy: blocked word", created_at: new Date(Date.now() - 20e3).toISOString(), edited: false, attachments: [] },
  ];
  server.automod = { blocked_terms: ["free gifts", "scam*"], block_links: false, max_mentions: 5, burst: { messages: 5, seconds: 10 }, exempt_roles: ["r2"] };
  server.commands = [{ bot: "m4", bot_name: "Tidy", commands: [
    { name: "remind", description: "Remind a member about something later", options: [
      { name: "who", description: "Who to remind", required: true, kind: "user" },
      { name: "minutes", description: "How many minutes from now", required: true, kind: "integer" },
      { name: "note", description: "What to say", required: false, kind: "string" },
    ] },
    { name: "roll", description: "Roll some dice", options: [{ name: "sides", description: "How many sides", required: false, kind: "integer" }] },
  ] }];
  mock.messages[server.channels[1].id] = [
    { id: "g0", channel: server.channels[1].id, author_id: "m2", author_name: "nova",
      content: "<sticker:party:party@mock>", created_at: new Date(Date.now() - 1200e3).toISOString(), edited: false, attachments: [] },
    { id: "g1", channel: server.channels[1].id, author_id: "m1", author_name: "ferris",
      content: "clutch round from last night", created_at: new Date(Date.now() - 900e3).toISOString(), edited: false,
      attachments: [{ filename: "clutch-round.mp4", size_bytes: 18400000, is_image: false, mime: "video/mp4", blob: "clip@mock", provider: null }] },
    { id: "g2", channel: server.channels[1].id, author_id: "m2", author_name: "nova",
      content: "full res screenshot of the new map", created_at: new Date(Date.now() - 600e3).toISOString(), edited: false,
      attachments: [{ filename: "new-map.png", size_bytes: 12900000, is_image: true, mime: "image/png", blob: "map@mock", provider: null }] },
    { id: "p1", channel: server.channels[1].id, author_id: "m2", author_name: "nova",
      content: "", created_at: new Date(Date.now() - 300e3).toISOString(), edited: false, attachments: [],
      poll: { question: "Raid night this week?", multi: false, total: 4, closed: false, closes_at: new Date(Date.now() + 3 * 3600e3).toISOString(),
        options: [{ text: "Friday", votes: 3, mine: true }, { text: "Saturday", votes: 1, mine: false }, { text: "Sunday after lunch", votes: 0, mine: false }] } },
    { id: "g3", channel: server.channels[1].id, author_id: "m1", author_name: "ferris",
      content: "", created_at: new Date(Date.now() - 300e3).toISOString(), edited: false,
      attachments: [{ filename: VOICE_MESSAGE_FILENAME, size_bytes: 61440, is_image: false, mime: VOICE_MESSAGE_MIME, blob: "voice@mock", provider: null }] },
    { id: "g8", channel: server.channels[1].id, author_id: "m2", author_name: "nova",
      content: "raid starts in five, grab your potions", created_at: new Date(Date.now() - 240e3).toISOString(), edited: false, attachments: [], tts: true },
    { id: "c1", channel: server.channels[1].id, author_id: "m2", author_name: "nova",
      content: "/roll 20", created_at: new Date(Date.now() - 240e3).toISOString(), edited: false, attachments: [],
      command: { bot: "m4", bot_name: "Tidy", name: "roll", options: { sides: "20" } } },
    { id: "c2", channel: server.channels[1].id, author_id: "m4", author_name: "Tidy",
      content: "nova rolled a 14 on a 20-sided die", created_at: new Date(Date.now() - 239e3).toISOString(), edited: false, attachments: [],
      reply_to: "c1", reply_preview: { author_id: "m2", author_name: "nova", excerpt: "/roll 20" } },
  ];
  mock.messages[server.channels[1].id].push(
    { id: "vi1", channel: server.channels[1].id, author_id: "m2", author_name: "nova", content: "",
      created_at: new Date(Date.now() - 240e3).toISOString(), edited: false, attachments: [], reactions: [],
      voice_invite: { server: server.id, channel: server.channels[4].id, invite_link: null, server_name: server.name, channel_name: server.channels[4].name } },
  );
  mock.dms.m1.messages.push(
    { id: "d5", from: "m1", from_name: "ferris", content: "", created_at: new Date(Date.now() - 500e3).toISOString(), outgoing: false,
      voice_invite: { server: crypto.randomUUID(), channel: crypto.randomUUID(), invite_link: `https://commy.chat/i/SYNTH2V4W#${"s".repeat(43)}`, server_name: "Synth Lounge", channel_name: "Jam Room" } },
  );
  mock.readMarks = { [server.channels[1].id]: { id: "g0", at: Date.now() - 1200e3, set: Date.now() - 1100e3 } };
  (mock.voiceClips ||= {})["voice@mock"] = 14;
  mock.listPins = { dms: ["m1"], channels: [{ server: server.id, channel: server.channels[1].id }] };
  const g = server.channels[1].id;
  const ref = (author_id, author_name, excerpt) => ({ author_id, author_name, excerpt });
  mock.messages[g].push(
    { id: "g7", channel: g, author_id: "m1", author_name: "ferris", content: "party time :ferris_wave:", created_at: new Date(Date.now() - 1100e3).toISOString(), edited: false, attachments: [],
      reply_to: "g0", reply_preview: ref("m2", "nova", "Sticker") },
    { id: "g3", channel: g, author_id: "m2", author_name: "nova", content: "that flick at the end :ferris:", created_at: new Date(Date.now() - 800e3).toISOString(), edited: false, attachments: [],
      reply_to: "g1", reply_preview: ref("m1", "ferris", "clutch round from last night") },
    { id: "g4", channel: g, author_id: "m1", author_name: "ferris", content: "pure luck, honestly", created_at: new Date(Date.now() - 700e3).toISOString(), edited: false, attachments: [],
      reply_to: "g3", reply_preview: ref("m2", "nova", "that flick at the end :ferris:") },
    { id: "g5", channel: g, author_id: "m3", author_name: "sleepy", content: "the water shader looks off on the east side", created_at: new Date(Date.now() - 500e3).toISOString(), edited: false, attachments: [],
      reply_to: "g2", reply_preview: ref("m2", "nova", "full res screenshot of the new map") },
    { id: "g6", channel: g, author_id: "m2", author_name: "nova", content: "known issue, the fix lands with the next patch", created_at: new Date(Date.now() - 400e3).toISOString(), edited: false, attachments: [],
      reply_to: "g5", reply_preview: ref("m3", "sleepy", "the water shader looks off on the east side") },
  );
  server.channels[1].threads = [
    { root: "g2", name: "Map feedback", archived: false, locked: false, by: "m2" },
    { root: "g1", name: "Clip of the week", archived: false, locked: true, by: "m2" },
    { root: "g0", name: null, archived: true, locked: false, by: "m2" },
  ];
  mock.saved = {
    saved: [
      { message: "x2", server: server.id, server_name: server.name, channel: c0, channel_name: "welcome", dm: null, dm_name: null, author_id: "m2", author_name: "nova", content: "No central servers, no paywall. Everything peer to peer.", attachment: null, created_at: mock.messages[c0][1].created_at, saved_at: Date.now() - 600e3 },
      { message: "d4", server: null, server_name: null, channel: null, channel_name: null, dm: "m1", dm_name: "ferris", author_id: "m1", author_name: "ferris", content: "", attachment: "voice-fix-notes.txt", created_at: mock.dms.m1.messages[2].created_at, saved_at: Date.now() - 86400e3 },
    ],
    reminders: [
      { id: "r-x1", due: Date.now() + 50 * 60e3, entry: { message: "x1", server: server.id, server_name: server.name, channel: c0, channel_name: "welcome", dm: null, dm_name: null, author_id: "m1", author_name: "ferris", content: "Welcome to Commy, a private P2P chat app.", attachment: null, created_at: mock.messages[c0][0].created_at } },
    ],
  };
  mock.pins = [
    { channel: c0, message: "x2", by: "m1", at: new Date(Date.now() - 900e3).toISOString() },
    { channel: c0, message: "x1", by: "m1", at: new Date(Date.now() - 3000e3).toISOString() },
  ];
  const news = server.channels[3].id;
  mock.messages[news] = [
    { id: "n1", channel: news, author_id: "m1", author_name: "ferris",
      content: "Voice update is live. Noise cancellation now has a one-click toggle in the voice panel. Restart to pick it up.",
      created_at: new Date(Date.now() - 7200e3).toISOString(), edited: false, attachments: [], reactions: [{ emoji: "\u{1F389}", count: 3, users: ["m2", "m3", "me"] }] },
  ];
  mock.pins.push({ channel: news, message: "n1", by: "m1", at: new Date(Date.now() - 7000e3).toISOString() });
  server.awaited = 2;
  const ideas = ch("ideas", "forum", "Pitch a feature, report a bug or ask a question");
  ideas.tags = [
    { id: "ft1", name: "Feature", emoji: "\u{1F4A1}" },
    { id: "ft2", name: "Bug", emoji: "\u{1F41B}" },
    { id: "ft3", name: "Question", emoji: "\u2753" },
    { id: "ft4", name: "Voice", emoji: null },
  ];
  server.channels.push(ideas);
  const post = (id, author, name, content, minutesAgo, replyTo = null) => ({
    id, channel: ideas.id, author_id: author, author_name: name, content, created_at: new Date(Date.now() - minutesAgo * 60e3).toISOString(),
    edited: false, attachments: [], reactions: [], reply_to: replyTo,
  });
  mock.messages[ideas.id] = [
    post("f1", "m2", "nova", "It would be great to pick a **different sound** for each channel so raid pings stand out from chatter.\n\n- short clip per channel\n- falls back to the server sound", 2880),
    post("f1a", "m1", "ferris", "yes, and let the soundboard clips be picked too", 2700, "f1"),
    post("f1b", "m3", "sleepy", "as long as there is an off switch", 1500, "f1a"),
    post("f2", "m1", "ferris", "When two people share a screen in the same voice channel the second stream shows the first person's name until I click away. Windows build, both on the latest version.", 1440),
    post("f2a", "m2", "nova", "I can reproduce this. Looks like the label only updates on the first frame.", 1380, "f2"),
    post("f3", "m3", "sleepy", "Is there a way to keep the overlay on a second monitor? Mine always lands on the game screen.", 600),
    post("f3a", "me", mock.user ? mock.user.username : "you", "Not yet, the overlay follows the game window. A corner picker per monitor is on the list.", 540, "f3"),
    post("f3b", "m3", "sleepy", "thanks, that works for now", 90, "f3a"),
    post("f4", "me", mock.user ? mock.user.username : "you", "Noise cancellation eats the first syllable when I start talking after a long pause. Toggling it off and on fixes it until the next pause.", 240),
    post("f5", "m2", "nova", "Can we get a quick poll for picking raid nights? Reactions work but counting them is a chore.", 45),
  ];
  mock.forumPosts = { ...(mock.forumPosts || {}), [ideas.id]: [
    { message: "f1", title: "Per-channel notification sounds", tags: ["ft1"], author: "m2" },
    { message: "f2", title: "Second screen share shows the wrong name", tags: ["ft2", "ft4"], author: "m1" },
    { message: "f3", title: "Overlay on a second monitor?", tags: ["ft3"], author: "m3" },
    { message: "f4", title: "Noise cancellation clips the first word", tags: ["ft2", "ft4"], author: "me" },
    { message: "f5", title: "Polls for raid nights", tags: ["ft1"], author: "m2" },
  ] };
}

function mockMemberName(id) {
  if (id === "me") return mock.user ? mock.user.username : "you";
  const member = Object.values(mock.servers).flatMap(s => s.members).find(m => m.id === id);
  return member ? member.name : "Someone";
}

function mockSavedStore() {
  mock.saved ||= { saved: [], reminders: [] };
  return mock.saved;
}

function mockPinViews(channelId) {
  const server = Object.values(mock.servers)[0];
  const names = new Map((server ? server.channels : []).map(c => [c.id, c.name]));
  const byName = id => id === "me" ? (mock.user ? mock.user.username : "you") : ((server && server.members.find(m => m.id === id)) || { name: "Someone" }).name;
  return sortPins((mock.pins || []).filter(p => !channelId || p.channel === channelId))
    .map(p => ({ ...p, channel_name: names.get(p.channel) || "", by_name: byName(p.by), message: (mock.messages[p.channel] || []).find(m => m.id === p.message) }))
    .filter(p => p.message)
    .map(p => ({ ...p, message: { ...p.message, pinned: true } }));
}

function mockFrame(label, width, height, hue) {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d");
  const grad = ctx.createLinearGradient(0, 0, width, height);
  grad.addColorStop(0, `hsl(${hue % 360} 60% 35%)`);
  grad.addColorStop(1, `hsl(${(hue + 60) % 360} 60% 20%)`);
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, width, height);
  ctx.fillStyle = "rgba(255,255,255,.85)";
  ctx.font = `600 ${Math.round(height / 9)}px Inter, sans-serif`;
  ctx.fillText(label, width * 0.06, height * 0.2);
  ctx.font = `${Math.round(height / 14)}px Inter, sans-serif`;
  ctx.fillText(new Date().toLocaleTimeString(), width * 0.06, height * 0.32);
  return canvas.toDataURL("image/jpeg", 0.7).split(",")[1];
}

function mockTemplateView(id, title, blurb, layout, roles = [], hasRules = false) {
  const channels = [];
  for (const [category, entries] of layout) {
    const parent = category ? channels.push({ name: category, kind: "category", category: null }) - 1 : null;
    for (const entry of entries) {
      const [kind, name] = entry.startsWith("v:") ? ["voice", entry.slice(2)] : entry.startsWith("a:") ? ["announcement", entry.slice(2)] : ["text", entry];
      channels.push({ name, kind, category: parent });
    }
  }
  return { id, title, blurb, name: title, channels, roles: roles.map(([name, color]) => ({ name, color })), has_rules: hasRules };
}

function mockEmojiFace(hue, mouth, eyes) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">`
    + `<circle cx="64" cy="64" r="58" fill="hsl(${hue} 80% 62%)"/>`
    + `<circle cx="44" cy="54" r="${eyes}" fill="#1b1e2e"/><circle cx="84" cy="54" r="${eyes}" fill="#1b1e2e"/>`
    + `<path d="${mouth}" stroke="#1b1e2e" stroke-width="7" fill="none" stroke-linecap="round"/></svg>`;
  return `data:image/svg+xml;base64,${btoa(svg)}`;
}

function mockEmojiCatalog(query = "", category = null, page = 0) {
  const names = ["party_blob", "cat_wave", "pog", "sad_frog", "heart_eyes", "thumbs_up_cat", "gg_wp", "blob_dance", "clap", "pepe_think",
    "shy", "sparkle_heart", "wave_hi", "no_thanks", "cool_cat", "salute", "pixel_heart", "sleepy", "blush_blob", "hype",
    "laugh_cry", "rage", "duck_wave", "sus", "yes", "nope", "pat", "bonk", "hug_blob", "snack"];
  const mouths = ["M40 84 Q64 108 88 84", "M42 92 Q64 76 86 92", "M44 88 L84 88", "M48 82 Q64 98 80 82"];
  const items = names.map((title, i) => ({
    id: 6000 - i, title, url: mockEmojiFace((i * 37) % 360, mouths[i % mouths.length], 6 + (i % 4) * 2), animated: i % 7 === 0, category: [22, 15, 3, 10, 17][i % 5],
  }));
  const q = String(query || "").toLowerCase().trim();
  const matches = items.filter(e => (!q || e.title.includes(q)) && (!category || e.category === Number(category)));
  const categories = [[22, "Reactions"], [15, "Cute"], [3, "Meme"], [10, "Gaming"], [8, "Animated"], [17, "Animals"], [4, "Anime"], [6, "Blobs"], [20, "Hearts"], [21, "Pixel Art"]]
    .map(([id, name]) => ({ id, name }));
  const start = (page || 0) * 60;
  return { results: matches.slice(start, start + 60).map(({ id, title, url, animated }) => ({ id, title, url, animated })), more: matches.length > start + 60, categories };
}

function mockStickerSearch(query = "", from = "stickers") {
  const gifs = from === "gifs";
  const names = gifs
    ? ["Mic Drop", "Slow Clap", "Facepalm", "Dancing", "Mind Blown", "Eye Roll", "Nodding", "Shrug", "Cheers", "Sleepy", "Excited", "Nope"]
    : ["Happy Dance", "Sleepy Cat", "Thumbs Up", "Love You", "Party Time", "Good Morning", "So Sad", "Wow", "High Five", "Bye Bye", "Hugs", "LOL"];
  const q = String(query || "").toLowerCase().trim();
  const results = names
    .map((title, i) => {
      const url = mockEmojiFace((i * 53 + (gifs ? 200 : 20)) % 360, ["M40 84 Q64 108 88 84", "M48 82 Q64 98 80 82"][i % 2], 8);
      return { id: `mock-${from}-${i}`, title, preview: url, url, ratio: 1 };
    })
    .filter(s => !q || s.title.toLowerCase().includes(q));
  return { results, more: false };
}

function mockTemplates() {
  return [
    mockTemplateView("fresh", "Start fresh", "A chat channel and a voice room", [[null, ["general", "v:General", "v:AFK"]]]),
    mockTemplateView("friends", "Friends", "Photos, memes and plans", [[null, ["general", "photos", "memes", "plans", "v:Hangout", "v:Gaming", "v:Movie Night", "v:AFK"]]]),
    mockTemplateView("gaming", "Gaming group", "Squads, clips and a lobby", [
      [null, ["general", "looking-for-group", "clips", "strategy", "v:Lobby", "v:Squad 1", "v:Squad 2", "v:Streaming", "v:AFK"]],
      ["News", ["a:announcements", "patch-notes"]],
    ], [["Squad Lead", "5b8cff"], ["Streamer", "a970ff"]]),
    mockTemplateView("community", "Community", "Welcome area and moderators", [
      [null, ["welcome", "a:announcements", "introductions", "general", "off-topic", "media", "help", "v:Lounge", "v:Events", "v:AFK"]],
      ["Staff", ["mod-chat"]],
    ], [["Moderator", "3ddc97"], ["Event Host", "ff9f43"], ["Events Ping", "5b8cff"]], true),
    mockTemplateView("study", "Study group", "Focus rooms and resources", [
      [null, ["general", "resources", "homework-help", "focus-log", "v:Study Room", "v:Quiet Room", "v:Break Room", "v:AFK"]],
    ], [["Tutor", "3ddc97"]]),
  ];
}

function mockTemplateFromCode(code) {
  try {
    return JSON.parse(atob(code.trim().slice("commy-template:".length)));
  } catch (_) {
    throw "that is not a server template code";
  }
}

function mockSafety() {
  return mock.safety || (mock.safety = { blocked: [], dm_policy: "everyone", spam_filter: true, profile_policy: "everyone" });
}

function mockFriends() {
  return mock.friends || (mock.friends = {
    friends: [{ id: "m1", name: "ferris" }, { id: "m3", name: "sleepy" }],
    incoming: [{ id: "m2", name: "nova", note: "We were in the Hangout call last night, the one where we planned Friday's raid" }],
    outgoing: [],
  });
}

function mockProfileFor(member) {
  const friend = mockFriends().friends.some(f => f.id === member.id);
  return publicProfile(member, canSeeProfile(member.profile_policy, { friend, sharesServer: true }));
}

function mockChatRestricted(server, field) {
  return mock.plainMember && Boolean(server.chat_restricted && server.chat_restricted[field]);
}

function mockServerOfChannel(channelId) {
  return Object.values(mock.servers).find(s => s.channels.some(c => c.id === channelId));
}

function mockCanSetVoiceStatus(channel) {
  return channel.kind === "voice" && (!mock.plainMember || (channel.voice_user_ids || []).includes("me"));
}

async function mockInvoke(cmd, args = {}) {
  switch (cmd) {
    case "get_user": return mock.user;
    case "init_user":
      mock.user = { id: "me", username: args.username, display_name: null, presence: "online", custom_status: null };
      if (Object.keys(mock.servers).length === 0) mockSeed();
      return mock.user;
    case "list_servers":
      return Object.values(mock.servers).map(s => ({ id: s.id, name: s.name, member_count: s.members.length, icon: s.icon || null, can_invite: !mock.plainMember, group: Boolean(s.group) }));
    case "create_group_dm": {
      const members = Object.values(mock.servers).flatMap(s => s.members).filter(m => args.userIds.includes(m.id));
      const seen = new Set();
      const unique = members.filter(m => !seen.has(m.id) && seen.add(m.id));
      const group = {
        id: crypto.randomUUID(), group: true, owner: "me",
        name: unique.map(m => m.name).sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase())).join(", "),
        channels: [{ id: crypto.randomUUID(), name: "chat", kind: "text", topic: null, voice_users: [] }, { id: crypto.randomUUID(), name: "call", kind: "voice", topic: null, voice_users: [] }],
        members: unique.map(m => ({ ...m })),
      };
      mock.servers[group.id] = group;
      return { id: group.id, name: group.name, member_count: group.members.length + 1, icon: null, can_invite: true, group: true };
    }
    case "add_group_members": {
      const group = mock.servers[args.serverId];
      if (!group || !group.group) throw "that is not a group you are in";
      const members = Object.values(mock.servers).flatMap(s => s.members).filter(m => args.userIds.includes(m.id));
      for (const m of members) if (!group.members.some(x => x.id === m.id)) group.members.push({ ...m });
      group.name = group.members.map(m => m.name).sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase())).join(", ");
      return null;
    }
    case "get_server": {
      const s = mock.servers[args.serverId];
      if (!s) throw "unknown server";
      const members = s.members.map(m => mockProfileFor({ username: m.username || m.name, nickname: m.nickname || null, priority_speaker: false, ...m }));
      if (mock.user && !members.some(m => m.id === "me")) {
        const presence = mock.user.presence === "invisible" ? "offline" : mock.user.presence || "online";
        const me = { id: "me", name: s.myNickname || mock.user.username, username: mock.user.username, nickname: s.myNickname || null, presence, custom_status: mock.user.custom_status || null, status_emoji: mock.user.status_emoji || null, about: mock.user.about || null, roles: s.myRoles || [], can_mention_everyone: !mock.plainMember };
        const vc = s.channels.find(c => isVoiceKind(c.kind) && (c.voice_user_ids || []).includes("me"));
        if (vc) {
          const at = vc.voice_user_ids.indexOf("me");
          Object.assign(me, { voice_channel: vc.id, voice_since: vc.voice_since[at], voice_muted: vc.voice_muted[at], voice_deafened: vc.voice_deafened[at] });
        }
        members.unshift(me);
      }
      return {
        ...s, members, screen_shares: [], cameras: mock.cameraOn ? ["me"] : [], roles: s.roles || [], afk_channel: s.afk_channel ?? null,
        game_tags: s.game_tags || [], events: s.events || [], rules: s.rules || null, needs_rules: Boolean(s.needs_rules), can_manage_events: !mock.plainMember, onboarding: s.onboarding || null,
        chat_restricted: { embed_links: false, read_history: false, create_threads: false, send_polls: false, create_events: false, ...(s.chat_restricted || {}) },
        can_embed_links: !mockChatRestricted(s, "embed_links"), can_read_history: !mockChatRestricted(s, "read_history"), can_create_threads: !mockChatRestricted(s, "create_threads"),
        can_send_polls: !mockChatRestricted(s, "send_polls"), can_create_events: !mockChatRestricted(s, "create_events"),
        channels: s.channels
          .map((c, i) => {
            const admits = ids => !(ids || []).length || ids.some(id => (s.myRoles || []).includes(id));
            const sees = admits(c.view_roles);
            const sendable = admits(c.send_roles);
            const view = { parent: null, position: i, user_limit: 0, slow_mode: 0, tags: [], threads: [], speakers: [], hands: [], pins: (mock.pins || []).filter(p => p.channel === c.id).length, view_roles: [], send_roles: [], voice_status: null, ...c, can_view: sees, can_send: sees && sendable, key_ready: sees, can_set_voice_status: mockCanSetVoiceStatus(c) };
            view.posts = c.kind === "forum" && sees ? forumPostViews((mock.forumPosts || {})[c.id] || [], mock.messages[c.id] || [], mockMemberName) : [];
            return view;
          })
          .filter(c => c.can_view || !mock.plainMember),
        can_manage_server: !mock.plainMember, can_manage_channels: !mock.plainMember, can_manage_roles: !mock.plainMember, can_kick: !mock.plainMember, can_manage_messages: !mock.plainMember, can_bypass_slow_mode: !mock.plainMember,
        emojis: s.emojis || [], sounds: s.sounds || [], emojis_shareable: !!s.emojis_shareable, soundboard_open: !!s.soundboard_open, sounds_shareable: !!s.sounds_shareable, public: !!s.public,
        can_manage_expressions: !mock.plainMember, can_upload_sounds: !mock.plainMember, can_use_soundboard: true, uploads_restricted: !!s.uploads_restricted, can_attach_files: !s.uploads_restricted || !mock.plainMember,
        can_send_voice_messages: !s.voice_messages_restricted || !mock.plainMember,
        can_send_tts_messages: !s.tts_restricted || !mock.plainMember,
        stickers: s.stickers || [], emoticons: s.emoticons || [], stickers_shareable: !!s.stickers_shareable, emoticons_shareable: !!s.emoticons_shareable,
        add_restricted: { emojis: false, stickers: false, emoticons: false, ...(s.add_restricted || {}) },
        can_add_emojis: !mock.plainMember || !(s.add_restricted || {}).emojis, can_add_stickers: !mock.plainMember || !(s.add_restricted || {}).stickers,
        can_add_emoticons: !mock.plainMember || !(s.add_restricted || {}).emoticons, can_use_external: s.can_use_external !== false,
        can_mute: !mock.plainMember, can_move: !mock.plainMember, can_manage_stage: !mock.plainMember, can_ban: !mock.plainMember, can_time_out: !mock.plainMember, my_timeout_until: s.my_timeout_until || null, can_manage_nicknames: !mock.plainMember, can_change_nickname: true, can_invite: !mock.plainMember, can_pin: !mock.plainMember, can_post_announcements: !mock.plainMember, can_view_audit_log: !mock.plainMember, server_muted: s.server_muted || [], server_deafened: s.server_deafened || [], banned: s.banned || [],
        automod: mock.plainMember ? null : s.automod || { blocked_terms: [], block_links: false, max_mentions: null, burst: null, exempt_roles: [] },
      };
    }
    case "create_server": {
      const template = looksLikeTemplateCode(args.template || "") ? mockTemplateFromCode(args.template)
        : mockTemplates().find(t => t.id === (args.template || "fresh"));
      const ids = template.channels.map(() => crypto.randomUUID());
      const server = {
        id: crypto.randomUUID(), name: args.name, owner: "me",
        channels: template.channels.map((c, i) => ({ id: ids[i], name: c.name, kind: c.kind, topic: null, parent: c.category == null ? null : ids[c.category], position: i, voice_users: [] })),
        roles: template.roles.map((r, i) => ({ id: crypto.randomUUID(), name: r.name, color: r.color, position: i + 1, permissions: [] })),
        rules: template.has_rules ? "Be kind." : null,
        members: [],
      };
      mock.servers[server.id] = server;
      return server;
    }
    case "list_server_templates": return mockTemplates();
    case "preview_server_template": return { ...mockTemplateFromCode(args.code), id: null };
    case "export_server_template": {
      const s = mock.servers[args.serverId];
      const index = new Map(s.channels.map((c, i) => [c.id, i]));
      const view = { title: s.name, blurb: "", name: s.name, has_rules: Boolean(s.rules),
        channels: s.channels.map(c => ({ name: c.name, kind: c.kind, category: c.parent ? index.get(c.parent) ?? null : null })),
        roles: (s.roles || []).filter(r => r.position).map(r => ({ name: r.name, color: r.color || null })) };
      return "commy-template:" + btoa(JSON.stringify(view));
    }
    case "join_server": throw "joining requires the desktop app";
    case "get_invite": return "commy" + "x".repeat(80) + " (mock invite, browser preview)";
    case "create_invite_link": {
      const hours = { "1h": 1, "1d": 24, "7d": 168, "30d": 720 }[args.expires || "7d"];
      if (!hours && !mock.servers[args.serverId]?.public) throw "Only links to a public server can last forever";
      const name = args.name ? checkInviteName(args.name) : null;
      if (name?.error) throw name.error;
      if (name && mockInvites().some(i => i.code === name.name && i.creator !== "me")) throw "That name is taken. Pick another one.";
      const code = name ? name.name : "KQ7M3N4P";
      const expiresAt = hours ? Date.now() + hours * 3600e3 : null;
      mock.invites = mockInvites().filter(i => i.code !== code).concat({ origin: "https://commy.chat", host: "commy.chat", code, custom: Boolean(name),
        creator: "me", creator_name: mock.user ? mock.user.username : "you", created_at: Date.now(), expires_at: expiresAt, can_revoke: true });
      return { url: `https://commy.chat/i/${code}#` + "q".repeat(name ? 22 : 43),
        expires_at: expiresAt ? new Date(expiresAt).toISOString() : null };
    }
    case "list_invites": return mockInvites().map(i => ({ ...i }));
    case "invite_full_link": return `${args.origin}/i/${args.code}#` + "q".repeat(/^[A-Z2-9]{8}$/.test(args.code) ? 43 : 22);
    case "revoke_invite": {
      if (!mockInvites().some(i => i.origin === args.origin && i.code === args.code)) throw "That invite is already gone";
      mock.invites = mock.invites.filter(i => !(i.origin === args.origin && i.code === args.code));
      return null;
    }
    case "account_server_info": return { origin: "https://commy.chat", host: "commy.chat", is_default: true };
    case "set_server_public": {
      const s = mock.servers[args.serverId];
      if (s) s.public = args.public;
      return null;
    }
    case "browse_directory": {
      if (mock.directoryMissing) return { host: "commy.chat", next_cursor: null, entries: [], available: false };
      const words = normalizeDirectorySearch(args.q).split(" ").filter(Boolean);
      const rows = (mock.directory || [])
        .filter(l => !args.category || l.category === args.category)
        .filter(l => words.every(w => `${l.name} ${l.description}`.toLowerCase().includes(w)))
        .sort((a, b) => b.members - a.members || a.name.localeCompare(b.name));
      const start = Number(args.cursor) || 0;
      const page = rows.slice(start, start + 24);
      return { host: "commy.chat", available: true, next_cursor: start + 24 < rows.length ? String(start + 24) : null,
        entries: page.map(l => ({ code: l.code, name: l.name, description: l.description, category: l.category, members: l.members, link: `https://commy.chat/i/${l.code}#${l.key}` })) };
    }
    case "publish_listing": {
      const s = mock.servers[args.serverId];
      if (!s) throw "unknown server";
      if (!s.public) throw "Make the server public before listing it in Discover";
      const existing = (mock.listings || {})[args.serverId];
      const code = existing ? existing.code : "KQ7M3N4P";
      const listing = { origin: "https://commy.chat", code, link: `https://commy.chat/i/${code}#${"q".repeat(43)}`, category: args.category,
        description: args.description, members: memberBucket(s.members.length), listed_at: Date.now() };
      mock.listings = { ...(mock.listings || {}), [args.serverId]: listing };
      mock.directory = (mock.directory || []).filter(l => l.code !== code)
        .concat({ code, key: "q".repeat(43), name: s.name, category: listing.category, members: listing.members, description: listing.description });
      return listing;
    }
    case "unlist_server": {
      const listing = (mock.listings || {})[args.serverId];
      if (!listing) throw "This server was not listed from this device";
      delete mock.listings[args.serverId];
      mock.directory = (mock.directory || []).filter(l => l.code !== listing.code);
      return null;
    }
    case "server_listing": return (mock.listings || {})[args.serverId] || null;
    case "resolve_invite_link": {
      const host = /commy:\/\/i\/([^/]+)\//.exec(args.link)?.[1] || new URL(args.link).host;
      const background = Object.values(mock.servers).map(s => s.invite_background).find(Boolean);
      return { server_name: "Pixel Pioneers", host, trusted: host === "commy.chat", account_server_host: "commy.chat",
        expires_at: new Date(Date.now() + 6 * 86400e3).toISOString(), ticket: "mock",
        background: background ? mock.serverImages?.[background] || null : null };
    }
    case "open_channel": (mock.channelOpens ||= {})[args.channelId] = Date.now(); return null;
    case "get_messages": {
      const kept = (mock.messages[args.channelId] || []).map(m => ({ ...m, pinned: (mock.pins || []).some(p => p.message === m.id) }));
      const historyOwner = mockServerOfChannel(args.channelId);
      const hidden = historyOwner && mockChatRestricted(historyOwner, "read_history");
      const all = hidden ? historyWindow(kept, (mock.channelOpens || {})[args.channelId]) : kept;
      if (!args.around) return all.slice(-100);
      const at = all.findIndex(m => m.id === args.around);
      if (at === -1) throw hidden && kept.some(m => m.id === args.around) ? "you can see only new messages in this channel" : "that message is no longer on this device";
      return all.slice(Math.max(0, at - 50), at + 51);
    }
    case "search_messages": return mockSearch(args);
    case "search_dms": return mockSearchDms(args);
    case "get_pins": return mockPinViews(args.channelId || null);
    case "saved_list": return mockSavedStore();
    case "sync_prefs_load": return {};
    case "sync_prefs_set": return null;
    case "saved_set": {
      const store = mockSavedStore();
      store.saved = store.saved.filter(s => s.message !== args.entry.message);
      if (args.saved) store.saved.unshift({ ...args.entry, saved_at: Date.now() });
      return store;
    }
    case "reminder_set": {
      const store = mockSavedStore();
      if (args.due <= Date.now()) throw "pick a time that is still ahead";
      store.reminders = store.reminders.filter(r => r.entry.message !== args.entry.message);
      store.reminders.push({ id: `${args.due}-${args.entry.message}`, due: args.due, entry: args.entry });
      store.reminders.sort((a, b) => a.due - b.due);
      return store;
    }
    case "reminder_cancel": {
      const store = mockSavedStore();
      store.reminders = store.reminders.filter(r => r.id !== args.id);
      return store;
    }
    case "pin_message": {
      mock.pins = (mock.pins || []).filter(p => p.message !== args.messageId);
      if (args.pinned) mock.pins.push({ channel: args.channelId, message: args.messageId, by: "me", at: new Date().toISOString() });
      return null;
    }
    case "send_poll": {
      if (mockChatRestricted(mockServerOfChannel(args.channelId) || {}, "send_polls")) throw "you do not have permission to create polls here";
      const closes_at = args.hours ? new Date(Date.now() + args.hours * 3600e3).toISOString() : null;
      const msg = {
        id: crypto.randomUUID(), channel: args.channelId, author_id: "me", author_name: mock.user.username, content: "",
        created_at: new Date().toISOString(), edited: false, attachments: [], reactions: [],
        poll: { question: args.question, options: args.options.map(text => ({ text, votes: 0, mine: false })), multi: args.multi, total: 0, closed: false, closes_at },
      };
      (mock.messages[args.channelId] ||= []).push(msg);
      return msg;
    }
    case "vote_poll": {
      const m = Object.values(mock.messages).flat().find(x => x.id === args.messageId);
      if (!m || !m.poll) throw "that message is not a poll";
      if (pollIsClosed(m.poll)) throw "that poll has closed";
      const votedBefore = m.poll.options.some(o => o.mine);
      m.poll.options = m.poll.options.map((o, i) => {
        const mine = args.choices.includes(i);
        return { ...o, mine, votes: o.votes + (mine ? 1 : 0) - (o.mine ? 1 : 0) };
      });
      m.poll.total += (args.choices.length > 0 ? 1 : 0) - (votedBefore ? 1 : 0);
      return null;
    }
    case "close_poll": {
      const m = Object.values(mock.messages).flat().find(x => x.id === args.messageId);
      if (!m || !m.poll) throw "that message is not a poll";
      if (m.author_id !== "me" && mock.plainMember) throw "only the poll's author or a moderator can close it";
      m.poll.closed = true;
      return null;
    }
    case "set_channel_access": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      c.view_roles = [...args.viewRoles];
      c.send_roles = [...args.sendRoles];
      return null;
    }
    case "set_channel_kind": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      c.kind = args.kind;
      return null;
    }
    case "set_thread": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      const root = (mock.messages[args.channelId] || []).find(m => m.id === args.rootId);
      if (!root) throw "that message no longer exists";
      if (mock.plainMember && root.author_id !== "me") throw "only the person who started a thread or a moderator can change it";
      if (!(c.threads || []).some(t => t.root === args.rootId) && mockChatRestricted(mock.servers[args.serverId], "create_threads")) throw "you do not have permission to start threads here";
      const name = (args.name || "").replace(/\s+/g, " ").trim() || null;
      c.threads = (c.threads || []).filter(t => t.root !== args.rootId);
      if (name || args.archived || args.locked) c.threads.push({ root: args.rootId, name, archived: Boolean(args.archived), locked: Boolean(args.locked), by: "me" });
      return null;
    }
    case "send_message": {
      const chan = Object.values(mock.servers).flatMap(s => s.channels).find(c => c.id === args.channelId);
      if (chan && chan.kind === "announcement" && mock.plainMember) throw "only members allowed to post announcements can write in this channel";
      if (chan && chan.slow_mode && mock.plainMember) {
        const mine = (mock.messages[args.channelId] || []).filter(m => m.author_id === "me");
        const last = mine.length ? Date.parse(mine[mine.length - 1].created_at) : 0;
        const left = Math.ceil((last + chan.slow_mode * 1000 - Date.now()) / 1000);
        if (left > 0) throw `slow mode is on here: you can send again in ${left}s`;
      }
      if (chan && args.replyTo) {
        const all = mock.messages[args.channelId] || [];
        const byId = new Map(all.map(m => [m.id, m]));
        const parent = byId.get(args.replyTo);
        const root = parent ? threadRootOf(parent, byId) : args.replyTo;
        const thread = (chan.threads || []).find(t => t.root === root);
        if (thread && thread.locked && mock.plainMember) throw "that thread is locked";
        if (thread) thread.archived = false;
      }
      if (args.tts && mock.plainMember && Object.values(mock.servers).some(s => s.tts_restricted && s.channels.includes(chan))) throw "you do not have permission to send text-to-speech messages here";
      const msg = {
        id: crypto.randomUUID(), channel: args.channelId, author_id: "me",
        author_name: mock.user.username, content: args.content,
        created_at: new Date().toISOString(), edited: false, attachments: [], reactions: [],
        tts: Boolean(args.tts),
        reply_to: args.replyTo || null,
        reply_preview: (() => { const o = (mock.messages[args.channelId] || []).find(m => m.id === args.replyTo); return o ? { author_id: o.author_id, author_name: o.author_name, excerpt: o.content.slice(0, 120) } : null; })(),
      };
      if (args.command) {
        const server = Object.values(mock.servers).find(s => s.channels.some(c => c.id === args.channelId));
        const group = ((server && server.commands) || []).find(g => g.bot === args.command.bot);
        if (!group || !group.commands.some(c => c.name === args.command.command)) throw "that bot has no such command, or an option does not fit";
        msg.command = { bot: args.command.bot, bot_name: group.bot_name, name: args.command.command, options: args.command.options || {} };
      }
      (mock.messages[args.channelId] ||= []).push(msg);
      return msg;
    }
    case "get_voice_settings": return mock.voice;
    case "set_auto_update": mock.voice.auto_update = args.enabled; return mock.voice;
    case "get_update_status": return mock.update || (mock.update = { current: "0.1.0", can_install: true, reason: null, phase: "idle", version: null, progress: null, checked_at: null, error: null, page: null });
    case "check_for_updates": mock.update = { ...mock.update, phase: "up_to_date", checked_at: Math.floor(Date.now() / 1000) }; return mock.update;
    case "restart_to_update": return null;
    case "set_noise_cancellation": mock.voice.noise_cancellation = args.enabled; return mock.voice;
    case "set_echo_cancellation": mock.voice.echo_cancellation = args.enabled; return mock.voice;
    case "set_auto_level": mock.voice.auto_level = args.enabled; return mock.voice;
    case "set_input_mode": Object.assign(mock.voice, { input_mode: args.mode, talk_key: args.talkKey, talk_release_ms: args.releaseMs }); return mock.voice;
    case "set_voice_hotkeys": Object.assign(mock.voice, { mute_hotkey: args.mute, deafen_hotkey: args.deafen }); return mock.voice;
    case "set_sensitivity": Object.assign(mock.voice, { auto_sensitivity: args.auto, sensitivity_db: args.thresholdDb }); return mock.voice;
    case "voice_hotkey_problems": return [];
    case "mic_level": return mock.micTesting ? -55 + 25 * Math.sin(Date.now() / 400) : -100;
    case "set_muted": mock.voice.muted = args.muted; return mock.voice;
    case "set_deafened":
      mock.voice.deafened = args.deafened;
      if (args.deafened) mock.voice.muted = true;
      return mock.voice;
    case "set_voice_channel": {
      const s = mock.servers[args.serverId];
      if (!s) return null;
      for (const c of s.channels) {
        if (!isVoiceKind(c.kind)) continue;
        const at = (c.voice_user_ids || []).indexOf("me");
        if (at >= 0) for (const k of ["voice_users", "voice_user_ids", "voice_since", "voice_muted", "voice_deafened"]) (c[k] || []).splice(at, 1);
        if (at >= 0 && !(c.voice_user_ids || []).length) c.voice_status = null;
        c.hands = (c.hands || []).filter(id => id !== "me");
        if (c.kind === "stage") c.speakers = (c.speakers || []).filter(id => id !== "me");
        if (c.id === args.channelId) {
          (c.voice_user_ids ||= []).push("me");
          (c.voice_since ||= []).push(Math.floor(Date.now() / 1000));
          (c.voice_muted ||= []).push(mock.voice.muted);
          (c.voice_deafened ||= []).push(mock.voice.deafened);
          c.voice_users.push(mock.user.display_name || mock.user.username);
          if (c.kind === "stage" && !mock.plainMember) c.speakers.push("me");
        }
      }
      return null;
    }
    case "voice_channel_presence": {
      const s = mock.servers[args.serverId];
      if (!s) return null;
      const c = s.channels.find(c => c.id === args.channelId);
      if (!c) throw "that voice channel is gone";
      const ids = c.voice_user_ids || [];
      return { server_name: s.name, channel_name: c.name, user_limit: c.user_limit || 0, can_view: true, users: (c.voice_users || []).map((name, i) => ({ id: ids[i] || null, name })) };
    }
    case "send_voice_invite":
    case "send_dm_voice_invite": {
      const s = mock.servers[args.serverId];
      const c = s && s.channels.find(c => c.id === args.channelId);
      if (!c || c.kind !== "voice") throw "that is not a voice channel";
      const voice_invite = { server: s.id, channel: c.id, invite_link: null, server_name: s.name, channel_name: c.name };
      if (cmd === "send_voice_invite") {
        const msg = { id: crypto.randomUUID(), channel: args.toChannelId, author_id: "me", author_name: mock.user.username, content: "",
          created_at: new Date().toISOString(), edited: false, attachments: [], reactions: [], voice_invite };
        (mock.messages[args.toChannelId] ||= []).push(msg);
        return msg;
      }
      const dm = mock.dms[args.userId];
      if (!dm) throw "no such conversation";
      if (!s.group && !s.members.some(m => m.id === args.userId)) voice_invite.invite_link = `https://commy.chat/i/KQ7M3N4P#${"q".repeat(43)}`;
      const sent = { id: crypto.randomUUID(), from: "me", from_name: mock.user.username, content: "", created_at: new Date().toISOString(),
        outgoing: true, pending: false, reply_to: null, reply_preview: null, reactions: [], attachments: [], voice_invite };
      dm.messages.push(sent);
      return sent;
    }
    case "set_stage_speaker": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      if (!c || c.kind !== "stage") throw "that is not a stage channel";
      c.speakers = (c.speakers || []).filter(id => id !== args.userId);
      if (args.speaker) {
        c.speakers.push(args.userId);
        c.hands = (c.hands || []).filter(id => id !== args.userId);
      }
      return null;
    }
    case "raise_hand": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.kind === "stage" && (ch.voice_user_ids || []).includes("me"));
      if (!c) throw "you are not in a stage channel here";
      if (args.raised && (c.speakers || []).includes("me")) throw "you already have the floor";
      c.hands = (c.hands || []).filter(id => id !== "me");
      if (args.raised) c.hands.push("me");
      return null;
    }
    case "set_activity": {
      const s = mock.servers[args.serverId];
      const channel = s && s.channels.find(c => c.kind === "voice" && (c.voice_user_ids || []).includes("me"));
      if (!channel) throw "join a voice channel first";
      if (args.activity && channel.activity && channel.activity.id !== args.activity.id && channel.activity.starter !== "me" && mock.plainMember) {
        throw "another activity is already running in this channel";
      }
      if (args.activity) channel.activity = args.activity;
      else delete channel.activity;
      return null;
    }
    case "send_activity_input": {
      const s = mock.servers[args.serverId];
      const channel = s && s.channels.find(c => c.kind === "voice" && (c.voice_user_ids || []).includes("me"));
      if (!channel || !channel.activity) throw "nothing is running in this channel";
      mockActivityOpponent(s, channel, args.input);
      return null;
    }
    case "list_dm_threads":
      return Object.entries(mock.dms)
        .map(([user, d]) => {
          const last = d.messages[d.messages.length - 1];
          return {
            user, name: d.name,
            last_message: last ? (last.outgoing ? "You: " : "") + last.content : "",
            last_at: last ? last.created_at : "",
            kind: d.kind || "inbox",
          };
        })
        .concat(Object.values(mock.servers).filter(s => s.group).map(s => {
          const all = s.channels.filter(c => c.kind === "text").flatMap(c => mock.messages[c.id] || []);
          const last = all[all.length - 1];
          return {
            user: s.id, name: s.name,
            last_message: last ? `${last.author_id === "me" ? "You" : last.author_name}: ${last.content}` : "",
            last_at: last ? last.created_at : "",
            kind: "group",
          };
        }))
        .sort((a, b) => b.last_at.localeCompare(a.last_at));
    case "get_dm_messages": return (mock.dms[args.userId] || { messages: [] }).messages;
    case "open_dm": {
      const member = Object.values(mock.servers).flatMap(s => s.members).find(m => m.id === args.userId);
      mock.dms[args.userId] ||= { name: member ? member.name : "Unknown User", messages: [] };
      return { user: args.userId, name: mock.dms[args.userId].name, last_message: "", last_at: "", kind: "inbox" };
    }
    case "send_dm":
    case "attach_dm_bytes": {
      const dm = mock.dms[args.userId];
      if (!dm) throw "no such conversation";
      const partner = Object.values(mock.servers).flatMap(s => s.members).find(m => m.id === args.userId);
      const original = args.replyTo ? dm.messages.find(m => m.id === args.replyTo) : null;
      const sent = {
        id: crypto.randomUUID(), from: "me", from_name: mock.user.username, content: args.content || "",
        created_at: new Date().toISOString(), outgoing: true, pending: !partner || partner.presence === "offline",
        reply_to: original ? original.id : null,
        reply_preview: original ? { author_id: original.from, author_name: original.from_name, excerpt: original.content.slice(0, 120) } : null,
        reactions: [],
        attachments: cmd === "attach_dm_bytes"
          ? [{ filename: args.filename, size_bytes: Math.round(args.data.length * 3 / 4), is_image: /\.(png|jpe?g|gif|webp)$/i.test(args.filename), mime: "application/octet-stream", blob: "mock" + Date.now(), provider: null }]
          : [],
      };
      dm.messages.push(sent);
      return sent;
    }
    case "attach_dm_file": return null;
    case "start_voice_recording":
      mock.recordingSince = Date.now();
      return null;
    case "cancel_voice_recording":
      mock.recordingSince = null;
      return null;
    case "send_voice_message":
    case "send_dm_voice_message": {
      const secs = Math.max(1, Math.round((Date.now() - (mock.recordingSince || Date.now())) / 1000));
      mock.recordingSince = null;
      const att = mockVoiceAttachment(secs);
      if (cmd === "send_dm_voice_message") {
        const dm = mock.dms[args.userId];
        if (!dm) throw "unknown conversation";
        const sent = { id: crypto.randomUUID(), from: "me", from_name: mock.user.username, content: "", created_at: new Date().toISOString(), outgoing: true, pending: false, reply_to: null, reply_preview: null, reactions: [], attachments: [att] };
        dm.messages.push(sent);
        return sent;
      }
      const sent = { id: crypto.randomUUID(), channel: args.channelId, author_id: "me", author_name: mock.user.username, content: "", created_at: new Date().toISOString(), edited: false, attachments: [att], reactions: [] };
      (mock.messages[args.channelId] ||= []).push(sent);
      return sent;
    }
    case "get_voice_message": return mockOgg((mock.voiceClips || {})[args.blob] ?? 30);
    case "start_dm_call": {
      const dm = mock.dms[args.userId];
      if (!dm) throw "no such conversation";
      return { state: "ringing", user: args.userId, name: dm.name, outgoing: true, seconds: 0, reason: null };
    }
    case "answer_dm_call":
    case "end_dm_call":
    case "get_dm_call":
      return null;
    case "edit_dm": {
      const m = (mock.dms[args.userId] || { messages: [] }).messages.find(x => x.id === args.messageId);
      if (!m || !m.outgoing) throw "you can only edit your own messages";
      m.content = args.content;
      m.edited = true;
      return null;
    }
    case "delete_dm": {
      const dm = mock.dms[args.userId];
      if (!dm) throw "no such conversation";
      dm.messages = dm.messages.filter(m => m.id !== args.messageId || !m.outgoing);
      return null;
    }
    case "set_dm_reaction": {
      const m = (mock.dms[args.userId] || { messages: [] }).messages.find(x => x.id === args.messageId);
      if (!m) throw "that message no longer exists";
      m.reactions = m.reactions || [];
      let r = m.reactions.find(x => x.emoji === args.emoji);
      if (args.active) {
        if (!r) m.reactions.push(r = { emoji: args.emoji, count: 0, users: [] });
        if (!r.users.includes("me")) r.users.push("me");
      } else if (r) r.users = r.users.filter(u => u !== "me");
      m.reactions = m.reactions.map(x => ({ ...x, count: x.users.length })).filter(x => x.count);
      return null;
    }
    case "update_profile":
      Object.assign(mock.user, {
        display_name: args.displayName || null,
        custom_status: args.customStatus || null,
        presence: args.presence,
      });
      if (args.statusEmoji !== undefined) mock.user.status_emoji = args.statusEmoji || null;
      return mock.user;
    case "set_avatar":
      if (args.kind === "image") {
        const blob = "mock" + Date.now();
        const mime = args.data.startsWith("R0lGOD") ? "image/gif" : args.data.startsWith("iVBOR") ? "image/png" : "image/jpeg";
        mock.avatars = { ...(mock.avatars || {}), [blob]: `data:${mime};base64,${args.data}` };
        mock.user.avatar = `blob:${blob}`;
      } else {
        mock.user.avatar = args.kind === "emoji" ? `emoji:${args.data}` : null;
      }
      return mock.user;
    case "set_profile_banner":
      if (args.data) {
        const blob = "mockbanner" + Date.now();
        mock.avatars = { ...(mock.avatars || {}), [blob]: `data:image/jpeg;base64,${args.data}` };
        mock.user.banner = `blob:${blob}`;
      } else {
        mock.user.banner = null;
      }
      return mock.user;
    case "rename_server": {
      mock.servers[args.serverId].name = args.name;
      const listing = (mock.listings || {})[args.serverId];
      if (listing) mock.directory = (mock.directory || []).map(l => l.code === listing.code ? { ...l, name: args.name } : l);
      return null;
    }
    case "create_channel":
      mock.servers[args.serverId].channels.push({
        id: crypto.randomUUID(), name: args.name, kind: args.kind, topic: null, voice_users: [],
      });
      return null;
    case "edit_message": {
      const m = (mock.messages[args.channelId] || []).find(x => x.id === args.messageId);
      m.content = args.content;
      m.edited = true;
      return null;
    }
    case "delete_message":
      mock.messages[args.channelId] = (mock.messages[args.channelId] || []).filter(x => x.id !== args.messageId);
      return null;
    case "update_channel": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      c.name = args.name;
      c.retention_days = args.retentionDays || null;
      return null;
    }
    case "set_channel_color": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      c.color = args.color || null;
      return null;
    }
    case "set_channel_sensitive": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      if (!holdsMessages(c)) throw "only text and forum channels can be age-restricted";
      c.sensitive = Boolean(args.sensitive);
      return null;
    }
    case "delete_channel": {
      const s = mock.servers[args.serverId];
      s.channels = s.channels.filter(c => c.id !== args.channelId);
      return null;
    }
    case "leave_server":
      delete mock.servers[args.serverId];
      return null;
    case "transfer_ownership": {
      const s = mock.servers[args.serverId];
      if (!s || args.confirmName.trim() !== s.name.trim()) throw "type the server name exactly as it is shown to hand it over";
      if (!s.members.some(m => m.id === args.userId)) throw "pick another member of this server";
      s.owner = args.userId;
      return null;
    }
    case "delete_server": {
      const s = mock.servers[args.serverId];
      if (!s || args.confirmName.trim() !== s.name.trim()) throw "type the server name exactly as it is shown to delete it";
      delete mock.servers[args.serverId];
      return null;
    }
    case "audit_log": return mockAuditLog();
    case "send_typing": return null;
    case "start_mic_test": mock.micTesting = true; return null;
    case "stop_mic_test": mock.micTesting = false; return null;
    case "set_afk_timeout": mock.voice.afk_timeout_minutes = args.minutes; return mock.voice;
    case "set_share_activity": mock.voice.share_activity = args.enabled; return mock.voice;
    case "list_audio_devices": return { inputs: ["Microphone (Realtek Audio)", "Headset Mic (HyperX Cloud)"], outputs: ["Speakers (Realtek Audio)", "Headphones (HyperX Cloud)"] };
    case "start_camera": mock.cameraOn = true; return null;
    case "notify": console.info("notify", args.title, args.body); return null;
    case "set_reaction": {
      const m = (mock.messages[args.channelId] || []).find(x => x.id === args.messageId);
      m.reactions = m.reactions || [];
      let r = m.reactions.find(x => x.emoji === args.emoji);
      if (args.active) {
        if (!r) m.reactions.push(r = { emoji: args.emoji, count: 0, users: [] });
        if (!r.users.includes("me")) r.users.push("me");
      } else if (r) r.users = r.users.filter(u => u !== "me");
      m.reactions = m.reactions.map(x => ({ ...x, count: x.users.length })).filter(x => x.count);
      return null;
    }
    case "set_server_image": {
      const ref = args.data ? `mock${Date.now()}@mock` : null;
      if (ref) mock.serverImages = { ...(mock.serverImages || {}), [ref]: `data:image/jpeg;base64,${args.data}` };
      mock.servers[args.serverId][args.which === "invite" ? "invite_background" : args.which] = ref;
      return null;
    }
    case "add_emoji": {
      const s = mock.servers[args.serverId];
      const ref = `mock${Date.now()}@mock`;
      mock.expressions = { ...(mock.expressions || {}), [ref]: `data:image/png;base64,${args.data}` };
      s.emojis = (s.emojis || []).filter(e => e.name !== args.name).concat({ name: args.name, image: ref });
      return null;
    }
    case "remove_emoji": {
      const s = mock.servers[args.serverId];
      s.emojis = (s.emojis || []).filter(e => e.name !== args.name);
      return null;
    }
    case "add_sticker": {
      const s = mock.servers[args.serverId];
      const ref = `mock${Date.now()}@mock`;
      mock.expressions = { ...(mock.expressions || {}), [ref]: `data:image/png;base64,${args.data}` };
      s.stickers = (s.stickers || []).filter(e => e.name !== args.name).concat({ name: args.name, image: ref });
      return null;
    }
    case "remove_sticker": {
      const s = mock.servers[args.serverId];
      s.stickers = (s.stickers || []).filter(e => e.name !== args.name);
      return null;
    }
    case "add_emoticon": {
      const s = mock.servers[args.serverId];
      s.emoticons = (s.emoticons || []).filter(e => e.name !== args.name).concat({ name: args.name, text: args.text.trim() });
      return null;
    }
    case "remove_emoticon": {
      const s = mock.servers[args.serverId];
      s.emoticons = (s.emoticons || []).filter(e => e.name !== args.name);
      return null;
    }
    case "set_expression_sharing": {
      const s = mock.servers[args.serverId];
      s.stickers_shareable = args.stickers;
      s.emoticons_shareable = args.emoticons;
      return null;
    }
    case "search_emoji_catalog": return mockEmojiCatalog(args.query, args.category, args.page);
    case "fetch_catalog_image": return args.url;
    case "sticker_search": return mockStickerSearch(args.query, args.from);
    case "store_catalog_sticker": {
      const blob = `ggs${Object.keys(mock.expressions || {}).length}@mock`;
      mock.expressions = { ...(mock.expressions || {}), [blob]: `data:image/png;base64,${args.data}` };
      return `<sticker:${args.name}:${blob}>`;
    }
    case "store_catalog_emoji": {
      const blob = `gg${Object.keys(mock.expressions || {}).length}@mock`;
      mock.expressions = { ...(mock.expressions || {}), [blob]: `data:image/png;base64,${args.data}` };
      return `<:${args.name}:${blob}>`;
    }
    case "add_sound": {
      const s = mock.servers[args.serverId];
      const ref = `mock${Date.now()}@mock`;
      mock.expressions = { ...(mock.expressions || {}), [ref]: `data:audio/mpeg;base64,${args.data}` };
      s.sounds = (s.sounds || []).filter(x => x.name !== args.name).concat({ name: args.name, blob: ref, volume: args.volume ?? 1, emoji: args.emoji || null });
      return null;
    }
    case "set_sound_emoji": {
      const s = mock.servers[args.serverId];
      const sound = (s.sounds || []).find(x => x.name === args.name);
      if (sound) sound.emoji = args.emoji || null;
      return null;
    }
    case "remove_sound": {
      const s = mock.servers[args.serverId];
      s.sounds = (s.sounds || []).filter(x => x.name !== args.name);
      return null;
    }
    case "set_expression_settings": {
      const s = mock.servers[args.serverId];
      s.emojis_shareable = args.emojisShareable;
      s.soundboard_open = args.soundboardOpen;
      return null;
    }
    case "play_soundboard": {
      const s = mock.servers[args.serverId];
      const builtin = defaultSound(args.name);
      if (builtin) {
        setTimeout(() => onSoundboardPlay({ server: s.id, channel: "", user: "me", name: args.name, blob: args.name, volume: 1 }), 0);
        return null;
      }
      const sound = (s.sounds || []).find(x => x.name === args.name);
      if (sound) setTimeout(() => onSoundboardPlay({ server: s.id, channel: "", user: "me", name: sound.name, blob: sound.blob, volume: sound.volume }), 0);
      return null;
    }
    case "fetch_expression": return (mock.expressions || {})[args.blob] || null;
    case "pick_sound": return null;
    case "is_mobile_app": return new URLSearchParams(location.search).has("mobile");
    case "set_role_access": {
      const srv = mock.servers[args.serverId];
      const chat = EVERYONE_CHAT_ACCESS.find(spec => spec.key === args.access);
      const perm = chat ? chat.permission : { files: "attach_files", emojis: "add_emojis", stickers: "add_stickers", emoticons: "add_emoticons" }[args.access];
      if (chat) srv.chat_restricted = { ...(srv.chat_restricted || {}), [chat.field]: args.restricted };
      else if (args.access === "files") srv.uploads_restricted = args.restricted;
      else srv.add_restricted = { ...(srv.add_restricted || {}), [args.access]: args.restricted };
      for (const r of srv.roles || []) {
        const has = (r.permissions || []).includes(perm);
        const want = chat ? args.restricted && !args.roleIds.includes(r.id) : args.restricted && args.roleIds.includes(r.id);
        if ((chat || args.restricted) && want !== has) r.permissions = want ? [...(r.permissions || []), perm] : r.permissions.filter(x => x !== perm);
      }
      return null;
    }
    case "set_sound_sharing": mock.servers[args.serverId].sounds_shareable = args.shareable; return null;
    case "set_activities_enabled": mock.servers[args.serverId].activities_enabled = args.enabled; return null;
    case "list_shareable_sounds":
      return Object.values(mock.servers).filter(s => s.sounds_shareable)
        .flatMap(s => (s.sounds || []).map(x => ({ server: s.id, server_name: s.name, name: x.name, blob: x.blob, volume: x.volume ?? 1, emoji: x.emoji || null })));
    case "play_shared_sound": return null;
    case "list_shareable_expressions": {
      const from = (flag, kind) => Object.values(mock.servers).filter(s => s[flag])
        .flatMap(s => (s[kind] || []).map(e => ({ server: s.id, server_name: s.name, ...e })));
      return { emojis: from("emojis_shareable", "emojis"), stickers: from("stickers_shareable", "stickers"), emoticons: from("emoticons_shareable", "emoticons") };
    }
    case "fetch_server_image": return (mock.serverImages || {})[args.image] || null;
    case "get_overlay_settings": return mock.overlay || (mock.overlay = { enabled: false, corner: "top-left", opacity: 0.9, show_chat: true, show_everyone: false, pictures_only: false, toggle_hotkey: "Ctrl+Shift+F10", chat_hotkey: "Ctrl+Shift+F11" });
    case "set_overlay_settings": mock.overlay = args.settings; return mock.overlay;
    case "overlay_hotkey_problems": return [];
    case "preview_overlay": return null;
    case "get_integrations": return mock.integrations || { igdb_client_id: "", igdb_configured: false };
    case "get_stream_deck": return mock.streamDeck || (mock.streamDeck = { enabled: false, status: "off", decks: [] });
    case "set_stream_deck_enabled":
      mock.streamDeck = { ...(mock.streamDeck || { decks: [] }), enabled: args.enabled, status: args.enabled ? "listening" : "off" };
      return mock.streamDeck;
    case "forget_stream_deck":
      mock.streamDeck.decks = mock.streamDeck.decks.filter(d => d.id !== args.id);
      return mock.streamDeck;
    case "answer_stream_deck_pairing": return null;
    case "set_igdb_credentials":
      mock.integrations = { igdb_client_id: args.clientId, igdb_configured: Boolean(args.clientId) };
      return args.clientId ? [{ id: 72, name: "Portal 2", cover: "co1rs4", year: 2011 }] : [];
    case "gif_search": return null;
    case "game_details":
      return { "hollowknight": { id: 14593, name: "Hollow Knight", cover: "co93cr", year: 2017, steam: 367520 }, "portal2": { id: 72, name: "Portal 2", cover: "co1rs4", year: 2011, steam: 620 } }[args.name.toLowerCase().replace(/[^a-z0-9]/g, "")] || null;
    case "search_games":
      return [{ id: 72, name: "Portal 2", cover: "co1rs4", year: 2011 }, { id: 71, name: "Portal", cover: "co1x7d", year: 2007 }]
        .filter(g => g.name.toLowerCase().includes(args.query.toLowerCase()));
    case "set_game_tags": mock.servers[args.serverId].game_tags = args.tags; mock.servers[args.serverId].game_tag_meta = args.meta || []; return null;
    case "set_rules": mock.servers[args.serverId].rules = args.rules; mock.servers[args.serverId].needs_rules = false; return null;
    case "set_automod": {
      const usage = termsUsage(args.rules.blocked_terms);
      if (usage.over) throw `keep it to ${AUTOMOD_MAX_TERMS} blocked words of up to ${AUTOMOD_MAX_TERM_CHARS} characters each, ${AUTOMOD_MAX_TERMS_BYTES} characters in all`;
      mock.servers[args.serverId].automod = { ...args.rules, blocked_terms: args.rules.blocked_terms.map(t => t.toLowerCase()) };
      return null;
    }
    case "accept_rules": mock.servers[args.serverId].needs_rules = false; return null;
    case "set_onboarding": mock.servers[args.serverId].onboarding = args.onboarding; return null;
    case "answer_onboarding": {
      const s = mock.servers[args.serverId];
      s.myRoles = (s.myRoles || []).filter(id => !args.revoke.includes(id));
      for (const id of args.grant) if (!s.myRoles.includes(id)) s.myRoles.push(id);
      return null;
    }
    case "save_event": {
      const s = mock.servers[args.serverId];
      if (!args.eventId && mockChatRestricted(s, "create_events")) throw "you do not have permission to create events";
      s.events = (s.events || []).filter(e => e.id !== args.eventId);
      const channel = s.channels.find(c => c.id === args.channelId);
      s.events.push({ id: args.eventId || crypto.randomUUID(), title: args.title, description: args.description, start: args.start, end: args.end,
        channel: args.channelId, channel_name: channel ? channel.name : null, creator: "me", creator_name: "commy", going: 0, interested: 0, my_rsvp: null, can_edit: true });
      s.events.sort((a, b) => a.start.localeCompare(b.start));
      return null;
    }
    case "delete_event": { const s = mock.servers[args.serverId]; s.events = (s.events || []).filter(e => e.id !== args.eventId); return null; }
    case "set_rsvp": {
      const e = (mock.servers[args.serverId].events || []).find(x => x.id === args.eventId);
      if (e.my_rsvp) e[e.my_rsvp] -= 1;
      e.my_rsvp = args.rsvp;
      if (args.rsvp) e[args.rsvp] += 1;
      return null;
    }
    case "create_category":
      mock.servers[args.serverId].channels.push({ id: crypto.randomUUID(), name: args.name, kind: "category", topic: null, voice_users: [], parent: null, position: 1000 + mock.servers[args.serverId].channels.length, user_limit: 0 });
      return null;
    case "move_channels": {
      const s = mock.servers[args.serverId];
      for (const m of args.moves) Object.assign(s.channels.find(c => c.id === m.channel), { parent: m.parent, position: m.position });
      return null;
    }
    case "set_channel_limit": mock.servers[args.serverId].channels.find(c => c.id === args.channelId).user_limit = args.limit; return null;
    case "set_channel_slow_mode": mock.servers[args.serverId].channels.find(c => c.id === args.channelId).slow_mode = args.seconds; return null;
    case "set_voice_status": mock.servers[args.serverId].channels.find(c => c.id === args.channelId).voice_status = cleanVoiceStatus(args.text) || null; return null;
    case "set_forum_tags": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      c.tags = args.tags.map(t => ({ id: t.id || crypto.randomUUID(), name: t.name, emoji: t.emoji || null }));
      for (const p of (mock.forumPosts || {})[c.id] || []) p.tags = p.tags.filter(t => c.tags.some(k => k.id === t));
      return null;
    }
    case "post_forum": {
      if (mock.plainMember && mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId).send_roles?.length) throw "only some roles can send messages in this channel";
      const msg = {
        id: crypto.randomUUID(), channel: args.channelId, author_id: "me", author_name: mock.user.username, content: args.content,
        created_at: new Date().toISOString(), edited: false, attachments: [], reactions: [], reply_to: null,
      };
      (mock.messages[args.channelId] ||= []).push(msg);
      ((mock.forumPosts ||= {})[args.channelId] ||= []).push({ message: msg.id, title: args.title, tags: [...args.tags], author: "me" });
      return msg;
    }
    case "edit_forum_post": {
      const p = ((mock.forumPosts || {})[args.channelId] || []).find(x => x.message === args.messageId);
      if (!p) throw "that post no longer exists";
      if (p.author !== "me" && mock.plainMember) throw "you can only change your own posts";
      p.title = args.title;
      p.tags = [...args.tags];
      return null;
    }
    case "update_role": {
      const r = (mock.servers[args.serverId].roles || []).find(x => x.id === args.roleId);
      Object.assign(r, { name: args.name, color: args.color, self_assignable: args.selfAssignable, hoist: !!args.hoist, tag: args.tag, mentionable: Boolean(args.mentionable) });
      if (args.permissions) r.permissions = args.permissions;
      return null;
    }
    case "list_friends": return mockFriends();
    case "friend_action": {
      const f = mockFriends();
      const name = (Object.values(mock.servers)[0].members.find(m => m.id === args.userId) || { name: "Someone" }).name;
      f.incoming = f.incoming.filter(x => x.id !== args.userId);
      if (args.action === "accept") f.friends.push({ id: args.userId, name });
      if (args.action === "request") f.outgoing.push({ id: args.userId, name });
      if (args.action === "remove") { f.friends = f.friends.filter(x => x.id !== args.userId); f.outgoing = f.outgoing.filter(x => x.id !== args.userId); }
      return null;
    }
    case "mock_presence": {
      const touched = [];
      for (const s of Object.values(mock.servers)) {
        for (const m of s.members) {
          if (m.id !== args.userId) continue;
          m.presence = args.presence;
          touched.push(s.id);
        }
      }
      return touched;
    }
    case "set_about": mock.user.about = args.about; return mock.user;
    case "set_window_title": return null;
    case "set_icons": mock.voice.app_icon = args.appIcon; mock.voice.tray_icon = args.trayIcon; return mock.voice;
    case "get_custom_icon": return null;
    case "choose_custom_icon": return null;
    case "set_pronouns": mock.voice.pronouns = (args.pronouns || "").trim() || null; return mock.voice;
    case "set_server_afk_timeout": mock.servers[args.serverId].afk_timeout_minutes = args.minutes; return null;
    case "voice_stats":
      return { codec: "Opus, 48 kHz mono, 10 ms frames", bitrate_kbps: 32, links: [
        { user: "m1", name: "ferris", rtt_ms: 18 + Math.random() * 6, loss_pct: 0.1, jitter_ms: 1.4, path: "direct", tx_bytes: (mock.tx = (mock.tx || 0) + 150000), rx_bytes: (mock.rx = (mock.rx || 0) + 140000) },
        { user: "m2", name: "nova", rtt_ms: 140, loss_pct: 2.3, jitter_ms: 6.8, path: "relay", tx_bytes: 0, rx_bytes: 0 },
      ] };
    case "account_status": return mock.account || (mock.account = { signed_in: true, user: { id: "u1", name: "Commy Commander", email: "commy.cmd@commy.chat" }, server: "https://commy.chat", server_host: "commy.chat", is_default_server: true, default_server_host: "commy.chat", server_from_env: false, device_linked: true, pending_code: null, server_gifs: true, server_games: true, server_emojis: true, server_stickers: true, server_media: false, unclaimed_profile: null, needs_account_folder: /[?&]claim\b/.test(location.search), others: [{ id: "u2", name: "Nova", email: "nova@example.test", server: "https://commy.chat", server_host: "commy.chat" }], max_accounts: 5 });
    case "account_open_folder": mock.account.unclaimed_profile = { username: "ferris", display_name: "Ferris", servers: 3 }; return mock.account;
    case "account_claim_profile": mock.account.unclaimed_profile = null; mock.account.needs_account_folder = false; return mock.account;
    case "account_begin_sign_in": return { user_code: "WDJB-MJHT", verification_url: "https://commy.chat/device?user_code=WDJBMJHT", expires_in: 600 };
    case "account_cancel_sign_in": return null;
    case "account_sign_out": mock.account.signed_in = false; mock.account.user = null; return mock.account;
    case "account_switch": {
      const next = mock.account.others.find(o => o.id === args.id);
      mock.account.others = [{ ...mock.account.user, server: mock.account.server, server_host: mock.account.server_host }, ...mock.account.others.filter(o => o !== next)];
      mock.account.user = { id: next.id, name: next.name, email: next.email };
      location.reload();
      return mock.account;
    }
    case "account_sign_out_other": mock.account.others = mock.account.others.filter(o => o.id !== args.id); return mock.account;
    case "set_account_server": if (args.server && !/^(https:\/\/)?[a-z0-9-]+(\.[a-z0-9-]+)*\.(com|org|net|chat)$/i.test(args.server)) throw "signin.server.bad.ending"; return mock.account;
    case "set_user_voice": {
      mock.voice.user_volumes = mock.voice.user_volumes || {};
      mock.voice.muted_users = mock.voice.muted_users || [];
      if (args.volume !== null) mock.voice.user_volumes[args.userId] = args.volume;
      if (args.muted !== null) mock.voice.muted_users = args.muted ? [...mock.voice.muted_users, args.userId] : mock.voice.muted_users.filter(u => u !== args.userId);
      return mock.voice;
    }
    case "set_nickname": {
      const s = mock.servers[args.serverId];
      const nickname = args.nickname || null;
      if (args.userId === "me") {
        s.myNickname = nickname;
        return null;
      }
      const m = s.members.find(x => x.id === args.userId);
      if (!m) throw "that person is not a member of this server";
      m.username = m.username || m.name;
      m.nickname = nickname;
      m.name = nickname || m.username;
      return null;
    }
    case "moderate_voice": {
      const s = mock.servers[args.serverId];
      s.server_muted = (s.server_muted || []).filter(u => u !== args.userId);
      s.server_deafened = (s.server_deafened || []).filter(u => u !== args.userId);
      if (args.muted) s.server_muted.push(args.userId);
      if (args.deafened) s.server_deafened.push(args.userId);
      return null;
    }
    case "move_voice_member": {
      const s = mock.servers[args.serverId];
      const from = s.channels.find(c => (c.voice_user_ids || []).includes(args.userId));
      if (!from) throw new Error("that member is not in voice here");
      const at = from.voice_user_ids.indexOf(args.userId);
      const carried = ["voice_users", "voice_user_ids", "voice_since", "voice_muted", "voice_deafened"].map(k => [k, (from[k] || []).splice(at, 1)[0]]);
      const to = args.channelId ? s.channels.find(c => c.id === args.channelId) : null;
      if (to) for (const [k, v] of carried) (to[k] ||= []).push(v);
      const member = s.members.find(x => x.id === args.userId);
      if (member) member.voice_channel = to ? to.id : null;
      return null;
    }
    case "ban_member": {
      const s = mock.servers[args.serverId];
      const m = s.members.find(x => x.id === args.userId);
      s.members = s.members.filter(x => x.id !== args.userId);
      s.banned = [...(s.banned || []), [args.userId, m ? m.name : "Someone"]];
      return null;
    }
    case "unban_member": { const s = mock.servers[args.serverId]; s.banned = (s.banned || []).filter(b => b[0] !== args.userId); return null; }
    case "time_out_member": {
      const m = mock.servers[args.serverId].members.find(x => x.id === args.userId);
      if (m) m.timeout_until = Date.now() + args.seconds * 1000;
      return null;
    }
    case "clear_timeout": {
      const m = mock.servers[args.serverId].members.find(x => x.id === args.userId);
      if (m) m.timeout_until = null;
      return null;
    }
    case "set_channel_banner": {
      const c = mock.servers[args.serverId].channels.find(ch => ch.id === args.channelId);
      const ref = args.data ? `mock${Date.now()}@mock` : null;
      if (ref) mock.serverImages = { ...(mock.serverImages || {}), [ref]: `data:image/jpeg;base64,${args.data}` };
      c.banner = ref;
      return null;
    }
    case "set_role_banner": {
      const r = (mock.servers[args.serverId].roles || []).find(x => x.id === args.roleId);
      const ref = args.data ? `mock${Date.now()}@mock` : null;
      if (ref) mock.serverImages = { ...(mock.serverImages || {}), [ref]: `data:image/jpeg;base64,${args.data}` };
      r.banner = ref;
      return null;
    }
    case "set_own_role": {
      const s = mock.servers[args.serverId];
      s.myRoles = (s.myRoles || []).filter(id => id !== args.roleId);
      if (args.held) s.myRoles.push(args.roleId);
      return null;
    }
    case "get_storage_settings": return mock.storage || (mock.storage = { keep: "90", servers: {} });
    case "set_history_window": {
      const s = mock.storage || (mock.storage = { keep: "90", servers: {} });
      if (!args.serverId) s.keep = args.keep;
      else if (args.keep) s.servers[args.serverId] = args.keep;
      else delete s.servers[args.serverId];
      return s;
    }
    case "get_safety": {
      const s = mockSafety();
      return { ...s, blocked: s.blocked.map(id => ({ id, name: (Object.values(mock.servers)[0].members.find(m => m.id === id) || { name: "Someone" }).name })) };
    }
    case "set_blocked": {
      const s = mockSafety();
      s.blocked = s.blocked.filter(id => id !== args.userId);
      if (args.blocked) s.blocked.push(args.userId);
      return null;
    }
    case "set_dm_settings": {
      const s = mockSafety();
      s.dm_policy = args.policy;
      s.spam_filter = args.spamFilter;
      return null;
    }
    case "set_profile_privacy": mockSafety().profile_policy = args.policy; return null;
    case "accept_dm_request": if (mock.dms[args.userId]) mock.dms[args.userId].kind = "inbox"; return null;
    case "decline_dm_request": delete mock.dms[args.userId]; return null;
    case "report_user": console.info("report", args); return null;
    case "list_reports": return mock.reports || [];
    case "dismiss_report": mock.reports = (mock.reports || []).filter(r => r.id !== args.reportId); return null;
    case "set_status_expiry":
      mock.voice.presence_until = args.presenceUntil;
      mock.voice.status_until = args.statusUntil;
      if (mock.user) mock.user.presence_until = args.presenceUntil;
      return mock.voice;
    case "set_unread": mock.unread = args.count; return null;
    case "read_state_load": return mockReadView(Object.keys(mock.readMarks || {}));
    case "read_state_set": mock.readMarks = mergeReadMaps(mock.readMarks, args.channels); return null;
    case "read_state_mark_read": {
      const server = mock.servers[args.serverId];
      const touched = [];
      for (const c of server ? server.channels : []) {
        if (args.channelIds && !args.channelIds.includes(c.id)) continue;
        const mark = latestMark(mock.messages[c.id] || []);
        if (!mark) continue;
        mock.readMarks = mergeReadMaps(mock.readMarks, { [c.id]: mark });
        touched.push(c.id);
      }
      return mockReadView(touched);
    }
    case "read_state_sync": return mockReadView(Object.keys(mock.readMarks || {}));
    case "pins_load": return mock.listPins || { dms: [], channels: [] };
    case "pin_dm": {
      const pins = mock.listPins || { dms: [], channels: [] };
      mock.listPins = { ...pins, dms: togglePinned(pins.dms, args.userId, args.pinned) };
      return mock.listPins;
    }
    case "pin_channel": {
      const pins = mock.listPins || { dms: [], channels: [] };
      const others = pins.channels.filter(c => !(c.server === args.serverId && c.channel === args.channelId));
      mock.listPins = { ...pins, channels: args.pinned ? others.concat({ server: args.serverId, channel: args.channelId }) : others };
      return mock.listPins;
    }
    case "get_awaited": return (mock.servers[args.serverId] || {}).awaited || 0;
    case "set_close_to_tray": mock.voice.close_to_tray = args.enabled; return mock.voice;
    case "autostart_allowed": return true;
    case "tray_prompt_due": return !mock.voice.tray_prompt_seen;
    case "finish_tray_prompt": mock.autostart = args.autostart; mock.voice.close_to_tray = args.closeToTray; mock.voice.tray_prompt_seen = true; return mock.voice;
    case "get_autostart": return !!mock.autostart;
    case "debug_log_status": return { verbose: !!mock.verboseLogs, saving: true };
    case "set_verbose_logging": mock.verboseLogs = args.enabled; return { verbose: !!mock.verboseLogs, saving: true };
    case "open_log_folder": return null;
    case "save_log_bundle": return "commy-logs.txt";
    case "run_connection_check":
      await new Promise(r => setTimeout(r, 600));
      return mock.connectionCheck || { quality: "good", direct: "open", relay_reachable: true, relay_rtt_ms: 38, ipv6: false };
    case "set_autostart": mock.autostart = args.enabled; return mock.autostart;
    case "stop_camera": mock.cameraOn = false; return null;
    case "push_camera_frame": return 1;
    case "watch_camera": return null;
    case "stop_watching_camera": return null;
    case "set_audio_devices": mock.voice.input_device = args.input; mock.voice.output_device = args.output; return mock.voice;
    case "set_afk_channel": mock.servers[args.serverId].afk_channel = args.channelId; return null;
    case "set_volumes":
      mock.voice.output_volume = args.outputVolume;
      mock.voice.input_gain = args.inputGain;
      return mock.voice;
    case "list_share_sources":
      return [
        { kind: "monitor", id: 1, name: "Display 1", is_primary: true },
        { kind: "monitor", id: 2, name: "Display 2", is_primary: false },
        { kind: "window", id: 10, name: "Visual Studio Code", is_primary: false },
        { kind: "window", id: 11, name: "Minecraft", is_primary: false },
        { kind: "window", id: 12, name: "Spotify", is_primary: false },
      ];
    case "share_source_preview": return mockFrame(`Source ${args.sourceId}`, 360, 203, args.sourceId * 47);
    case "start_screen_share": mock.share = { name: `Source ${args.sourceId}`, hue: args.sourceId * 47, audio: args.audio }; return null;
    case "share_audio_supported": return true;
    case "set_stream_volume": mock.voice.stream_volume = args.volume; return mock.voice;
    case "stop_screen_share": mock.share = null; return null;
    case "own_share_frame":
      if (!mock.share) throw "not sharing";
      return { width: 1280, height: 720, jpeg: mockFrame(mock.share.name, 640, 360, mock.share.hue), viewers: 2, audio: Boolean(mock.share.audio) };
    case "watch_screen":
      clearInterval(mock.watchTimer);
      mock.watchTimer = setInterval(() => {
        if (!state.watchingUser) return clearInterval(mock.watchTimer);
        showStageFrame(`data:image/jpeg;base64,${mockFrame("Remote stream", 960, 540, 200)}`, 1280, 720);
      }, 125);
      return null;
    case "stop_watching": clearInterval(mock.watchTimer); return null;
    case "create_role": {
      const roles = mock.servers[args.serverId].roles;
      const id = crypto.randomUUID();
      roles.push({ id, name: args.name.trim(), color: args.color, position: roles.length + 2, permissions: args.permissions, self_assignable: Boolean(args.selfAssignable), hoist: Boolean(args.hoist), tag: args.tag || null, mentionable: Boolean(args.mentionable) });
      return id;
    }
    case "delete_role":
    case "set_member_role":
    case "kick_member":
      return null;
    case "open_url":
      window.open(args.url, "_blank");
      return null;
    case "check_link": {
      const host = new URL(args.url).hostname;
      const trusted = (args.trusted || []).includes(host);
      return { url: args.url, host, shown_host: host, site: host, trust_key: host, level: trusted ? "open" : "ask", signs: [], can_trust: !trusted };
    }
    case "attach_file":
    case "download_attachment":
    case "note_user_activity": return null;
    case "get_attachment_video": throw "this feature needs the desktop app";
    case "get_attachment_preview":
    case "get_profile_image":
      if (mock.avatars && mock.avatars[args.blob]) return mock.avatars[args.blob];
      throw "this feature needs the desktop app";
    default: throw `unknown command ${cmd}`;
  }
}

const $ = id => document.getElementById(id);

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

const ICONS = {
  phone: "M6.6 10.8c1.4 2.8 3.8 5.1 6.6 6.6l2.2-2.2c.3-.3.7-.4 1-.2 1.1.4 2.3.6 3.6.6.6 0 1 .4 1 1V20c0 .6-.4 1-1 1-9.4 0-17-7.6-17-17 0-.6.4-1 1-1h3.5c.6 0 1 .4 1 1 0 1.3.2 2.5.6 3.6.1.3 0 .7-.2 1L6.6 10.8z",
  speaker: "M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8v8a4.5 4.5 0 0 0 2.5-4zM14 3.2v2.1a7 7 0 0 1 0 13.4v2.1a9 9 0 0 0 0-17.6z",
  mic: "M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.9V21h2v-3.1A7 7 0 0 0 19 11z",
  micOff: "M19 11h-1.7c0 .7-.2 1.4-.4 2l1.2 1.2c.6-.9.9-2 .9-3.2zm-4 .2V5a3 3 0 0 0-5.9-.8L15 10.1v1.1zM4.3 3 3 4.3l6 6V11a3 3 0 0 0 4.2 2.8l1.6 1.6A5 5 0 0 1 7 11H5a7 7 0 0 0 6 6.9V21h2v-3.1c.9-.1 1.8-.4 2.5-.9l4.2 4.2 1.3-1.3z",
  headset: "M12 3a9 9 0 0 0-9 9v7a2 2 0 0 0 2 2h3v-8H5v-1a7 7 0 0 1 14 0v1h-3v8h3a2 2 0 0 0 2-2v-7a9 9 0 0 0-9-9z",
  headsetOff: "M12 4a7 7 0 0 0-5.2 2.3L5.4 4.9A9 9 0 0 1 21 12v7c0 .3-.1.6-.2.9L19 18.1V14h-3v1.1L8.2 7.7A7 7 0 0 1 19 13v-1a7 7 0 0 0-7-7zM3.3 3 2 4.3l1.7 1.7A9 9 0 0 0 3 12v7a2 2 0 0 0 2 2h3v-8H5v-1c0-1.2.3-2.4.9-3.4L19.7 21 21 19.7z",
  chat: "M20 2H4a2 2 0 0 0-2 2v18l4-4h14a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2zm-2 12H6v-2h12zm0-3H6V9h12zm0-3H6V6h12z",
  plus: "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z",
  forum: "M4 3h16a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zm0 2v4h16V5H4zm0 8h16a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-4a2 2 0 0 1 2-2zm0 2v4h16v-4H4zm2-9h5v2H6V6zm0 10h8v2H6v-2z",
  grid: "M3 3h8v8H3V3zm2 2v4h4V5H5zm8-2h8v8h-8V3zm2 2v4h4V5h-4zM3 13h8v8H3v-8zm2 2v4h4v-4H5zm8-2h8v8h-8v-8zm2 2v4h4v-4h-4z",
  list: "M3 5h2v2H3V5zm4 0h14v2H7V5zM3 11h2v2H3v-2zm4 0h14v2H7v-2zM3 17h2v2H3v-2zm4 0h14v2H7v-2z",
  screen: "M20 3H4a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h5v2H7v2h10v-2h-2v-2h5a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zm0 13H4V5h16zm-9-2 5-3.5L11 7z",
  screenOff: "M21 16V5a2 2 0 0 0-2-2H6.8l2 2H19v10.2zM2.4 2.1 1.1 3.4 3 5.3V16a2 2 0 0 0 2 2h5v2H8v2h8v-2h-2v-2h3.7l3 3 1.3-1.3zM5 16V7.3l8.7 8.7z",
  hangup: "M12 9c-1.6 0-3.1.3-4.6.7v3.1c0 .4-.2.7-.6.9-1 .5-1.9 1.1-2.6 1.8a1 1 0 0 1-1.4 0L.3 13.1a1 1 0 0 1 0-1.4C3.3 8.8 7.5 7 12 7s8.7 1.8 11.7 4.7a1 1 0 0 1 0 1.4l-2.5 2.5a1 1 0 0 1-1.4 0c-.8-.7-1.6-1.3-2.6-1.8a1 1 0 0 1-.6-.9V9.7C15.1 9.3 13.6 9 12 9z",
  signal: "M2 22h3v-5H2zm5 0h3v-9H7zm5 0h3V9h-3zm5 0h3V2h-3z",
  noise: "M7 18h2V6H7zm4 4h2V2h-2zm-8-8h2v-4H3zm12 4h2V6h-2zm4-8v4h2v-4z",
  soundboard: "M12 3v10.55A4 4 0 1 0 14 17V7h4V3z",
  flag: "M14.4 6 14 4H5v17h2v-7h5.6l.4 2h7V6z",
  more: "M6 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4zm6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4z",
  block: "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM4 12a8 8 0 0 1 12.9-6.3L5.7 16.9A7.9 7.9 0 0 1 4 12zm8 8a7.9 7.9 0 0 1-4.9-1.7L18.3 7.1A8 8 0 0 1 12 20z",
  trash: "M6 19a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7H6zM19 4h-3.5l-1-1h-5l-1 1H5v2h14z",
  eyeOff: "M12 7a5 5 0 0 1 5 5 4.9 4.9 0 0 1-.36 1.83l2.92 2.92A11.8 11.8 0 0 0 22.99 12 11.83 11.83 0 0 0 11.99 4.5a11.6 11.6 0 0 0-4 .7l2.17 2.16A4.9 4.9 0 0 1 12 7zM2 4.27l2.28 2.28.46.46A11.8 11.8 0 0 0 1 12a11.82 11.82 0 0 0 15.4 6.66l.43.42L19.73 22 21 20.73 3.27 3zM7.53 9.8l1.55 1.55A2.8 2.8 0 0 0 9 12a3 3 0 0 0 3 3 2.8 2.8 0 0 0 .65-.08l1.55 1.55A5 5 0 0 1 7 12a4.9 4.9 0 0 1 .53-2.2zm4.31-.78 3.15 3.15.02-.16a3 3 0 0 0-3-3z",
  thread: "M4 4h16v12H5.17L4 17.17zm0-2a2 2 0 0 0-2 2v18l4-4h14a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2zm2 4h12v2H6zm0 4h9v2H6z",
  lock: "M7 8h1V6a4 4 0 0 1 8 0v2h1a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2v-9a2 2 0 0 1 2-2zm3 0h4V6a2 2 0 0 0-4 0zm-3 2v9h10v-9zm5 2.5a1.5 1.5 0 0 1 1.5 1.5 1.5 1.5 0 0 1-1.5 1.5 1.5 1.5 0 0 1-1.5-1.5 1.5 1.5 0 0 1 1.5-1.5z",
  archive: "M3 4h18v4H3zm2 2v2h14V6zM4 8h16v12a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1zm2 2v10h12V10zm3 2v2h6v-2z",
  reply: "M10 9V5l-7 7 7 7v-4.1c5 0 8.5 1.6 11 5.1-1-5-4-10-11-11z",
  smilePlus: "M20 4V2h-2v2h-2v2h2v2h2V6h2V4zM12 20a8 8 0 0 1 0-16 7.9 7.9 0 0 1 2 .26V2.2A10 10 0 1 0 21.8 10h-2.06A7.9 7.9 0 0 1 20 12a8 8 0 0 1-8 8zm-3.5-9A1.5 1.5 0 1 0 7 9.5 1.5 1.5 0 0 0 8.5 11zm7 0A1.5 1.5 0 1 0 14 9.5 1.5 1.5 0 0 0 15.5 11zm-8 3a5 5 0 0 0 9 0z",
  play: "M8 5v14l11-7z",
  pause: "M6 5h4v14H6zm8 0h4v14h-4z",
  send: "M3 20v-6l11-2-11-2V4l19 8z",
  folder: "M10 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-8z",
  camera: "M17 10.5V7a1 1 0 0 0-1-1H4a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-3.5l4 4v-11z",
  cameraOff: "M21 6.5l-4 4V7a1 1 0 0 0-1-1H9.82L21 17.18zM3.27 2 2 3.27 4.73 6H4a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h12a1 1 0 0 0 .55-.17L19.73 21 21 19.73z",
  gear: "M19.14 12.94c.04-.3.06-.61.06-.94s-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.49.49 0 0 0-.59-.22l-2.39.96a7 7 0 0 0-1.62-.94l-.36-2.54a.48.48 0 0 0-.48-.41h-3.84a.47.47 0 0 0-.47.41l-.36 2.54a7.4 7.4 0 0 0-1.62.94l-2.39-.96a.48.48 0 0 0-.59.22L2.74 8.87a.47.47 0 0 0 .12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54a7 7 0 0 0 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.48.48 0 0 0-.12-.61zM12 15.6A3.6 3.6 0 1 1 15.6 12a3.6 3.6 0 0 1-3.6 3.6z",
  pencil: "M3 17.25V21h3.75L17.8 9.94l-3.75-3.75zM20.7 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75z",
  person: "M12 6a2 2 0 1 1-2 2 2 2 0 0 1 2-2m0 10c2.7 0 5.8 1.29 6 2H6c.23-.72 3.31-2 6-2m0-12a4 4 0 1 0 4 4 4 4 0 0 0-4-4zm0 10c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z",
  friendAdd: "M15 12a4 4 0 1 0-4-4 4 4 0 0 0 4 4zm-9-2V7H4v3H1v2h3v3h2v-3h3v-2zm9 4c-2.67 0-8 1.34-8 4v2h16v-2c0-2.66-5.33-4-8-4z",
  serverAdd: "M13 7h-2v4H7v2h4v4h2v-4h4v-2h-4zm-1-5a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8z",
  smile: "M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8zM8.5 11A1.5 1.5 0 1 0 7 9.5 1.5 1.5 0 0 0 8.5 11zm7 0A1.5 1.5 0 1 0 14 9.5 1.5 1.5 0 0 0 15.5 11zm-8 3a5 5 0 0 0 9 0z",
  fullscreen: "M7 14H5v5h5v-2H7zm-2-4h2V7h3V5H5zm12 7h-3v2h5v-5h-2zM14 5v2h3v3h2V5z",
  expand: "M4 4h7v2H6v5H4zm16 16h-7v-2h5v-5h2z",
  collapse: "M11 11H4V9h5V4h2zm2 2h7v2h-5v5h-2z",
  calendar: "M19 4h-1V2h-2v2H8V2H6v2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2zm0 16H5V10h14zm0-12H5V6h14z",
  book: "M18 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2zM9 4h2v5l-1-.75L9 9zm9 16H6V4h1v9l3-2.25L13 13V4h5z",
  chevronLeft: "M15.4 7.4 14 6l-6 6 6 6 1.4-1.4L10.8 12z",
  chevronRight: "M8.6 16.6 10 18l6-6-6-6-1.4 1.4 4.6 4.6z",
  chevronDown: "M7.4 8.6 12 13.2l4.6-4.6L18 10l-6 6-6-6z",
  roles: "M21.41 11.41l-8.83-8.83A2 2 0 0 0 11.17 2H4a2 2 0 0 0-2 2v7.17a2 2 0 0 0 .59 1.42l8.83 8.83a2 2 0 0 0 2.82 0l7.17-7.17a2 2 0 0 0 0-2.84zM12.83 20 4 11.17V4h7.17L20 12.83zM6.5 5A1.5 1.5 0 1 0 8 6.5 1.5 1.5 0 0 0 6.5 5z",
  check: "M9 16.2 4.8 12l-1.4 1.4L9 19 21 7l-1.4-1.4z",
  poll: "M5 9.2h3V19H5zm5.6-4.2h2.8v14h-2.8zm5.6 8H19v6h-2.8z",
  close: "M18.4 4.2 12 10.6 5.6 4.2 4.2 5.6 10.6 12l-6.4 6.4 1.4 1.4 6.4-6.4 6.4 6.4 1.4-1.4-6.4-6.4 6.4-6.4z",
  eye: "M12 5C7 5 2.7 8.1 1 12.5 2.7 16.9 7 20 12 20s9.3-3.1 11-7.5C21.3 8.1 17 5 12 5zm0 12.5a5 5 0 1 1 5-5 5 5 0 0 1-5 5zm0-8a3 3 0 1 0 3 3 3 3 0 0 0-3-3z",
  pin: "M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z",
  pinOutline: "M14 4v5c0 1.12.37 2.16 1 3H9c.65-.86 1-1.9 1-3V4h4m3-2H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3V4h1c.55 0 1-.45 1-1s-.45-1-1-1z",
  pinOff: "M3.3 2 2 3.3l5.6 5.6V9c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7h1.17l6.96 6.96 1.3-1.3zM16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.44 0-.8.28-.94.67L16.7 13.3c.66-.78 1.76-1.3 2.3-1.3v-2c-1.66 0-3-1.34-3-3z",
  megaphone: "M18 11v2h4v-2h-4zm-2 6.61c.96.71 2.21 1.65 3.2 2.39.4-.53.8-1.07 1.2-1.6-.99-.74-2.24-1.68-3.2-2.4-.4.54-.8 1.08-1.2 1.61zM20.4 5.6c-.4-.53-.8-1.07-1.2-1.6-.99.74-2.24 1.68-3.2 2.4.4.53.8 1.07 1.2 1.6.96-.72 2.21-1.65 3.2-2.4zM4 9c-1.1 0-2 .9-2 2v2c0 1.1.9 2 2 2h1v4h2v-4h1l5 3V6L8 9H4zm11.5 3c0-1.33-.58-2.53-1.5-3.35v6.69c.92-.81 1.5-2.01 1.5-3.34z",
  uploadFile: "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zm4 18H6V4h7v5h5zM8 15.01l1.41 1.41L11 14.84V19h2v-4.16l1.59 1.59L16 15.01 12.01 11z",
  movie: "M4 6.47 5.76 10H20v8H4V6.47M22 4h-4l2 4h-3l-2-4h-2l2 4h-3l-2-4H8l2 4H7L5 4H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2z",
  gifBox: "M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zm0 16H5V5h14zM9.5 13v-1h1v1a1 1 0 0 1-1 1h-1a1 1 0 0 1-1-1v-2a1 1 0 0 1 1-1h1a1 1 0 0 1 1 1h-2v2zm3 1h-1v-4h1zm4-3h-2v.5H16v1h-1.5V14h-1v-4h3z",
  bellOff: "M12 22c1.1 0 2-.9 2-2h-4c0 1.1.9 2 2 2zm6-6v-5c0-3.07-1.63-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.64 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2zm-2 1H8v-6c0-2.48 1.51-4.5 4-4.5s4 2.02 4 4.5v6zM3.3 4.7l1.4-1.4 16 16-1.4 1.4z",
  lock: "M12 17a2 2 0 1 0 0-4 2 2 0 0 0 0 4zm6-9h-1V6A5 5 0 0 0 7 6v2H6a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V10a2 2 0 0 0-2-2zM8.9 6a3.1 3.1 0 0 1 6.2 0v2H8.9zM18 20H6V10h12z",
  people: "M16.5 13c-1.2 0-3.07.34-4.5 1-1.43-.67-3.3-1-4.5-1C5.33 13 1 14.08 1 16.25V19h22v-2.75c0-2.17-4.33-3.25-6.5-3.25zm-4 4.5h-10v-1.25c0-.54 2.56-1.75 5-1.75s5 1.21 5 1.75v1.25zm9 0H14v-1.25c0-.46-.2-.86-.52-1.22.88-.3 1.96-.53 3.02-.53 2.44 0 5 1.21 5 1.75v1.25zM7.5 12c1.93 0 3.5-1.57 3.5-3.5S9.43 5 7.5 5 4 6.57 4 8.5 5.57 12 7.5 12zm0-5.5c1.1 0 2 .9 2 2s-.9 2-2 2-2-.9-2-2 .9-2 2-2zm9 5.5c1.93 0 3.5-1.57 3.5-3.5S18.43 5 16.5 5 13 6.57 13 8.5s1.57 3.5 3.5 3.5zm0-5.5c1.1 0 2 .9 2 2s-.9 2-2 2-2-.9-2-2 .9-2 2-2z",
  gamepad: "M21 6H3c-1.1 0-2 .9-2 2v8c0 1.1.9 2 2 2h18c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2zm0 10H3V8h18v8zM6 15h2v-2h2v-2H8V9H6v2H4v2h2v2zm8.5-3a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3zm4-3a1.5 1.5 0 1 0 0 3 1.5 1.5 0 0 0 0-3z",
  paste: "M19 2h-4.18C14.4.84 13.3 0 12 0S9.6.84 9.18 2H5c-1.1 0-2 .9-2 2v16c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-7 0c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm7 18H5V4h2v3h10V4h2v16z",
  search: "M15.5 14h-.79l-.28-.27A6.47 6.47 0 0 0 16 9.5 6.5 6.5 0 1 0 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z",
  clock: "M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z",
  link: "M3.9 12c0-1.71 1.39-3.1 3.1-3.1h4V7H7a5 5 0 0 0 0 10h4v-1.9H7c-1.71 0-3.1-1.39-3.1-3.1zM8 13h8v-2H8zm9-6h-4v1.9h4c1.71 0 3.1 1.39 3.1 3.1s-1.39 3.1-3.1 3.1h-4V17h4a5 5 0 0 0 0-10z",
  download: "M18 15v3H6v-3H4v3c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2v-3zm-1-4-1.41-1.41L13 12.17V4h-2v8.17L8.41 9.59 7 11l5 5z",
  steam: "M11.979 0C5.678 0 .511 4.86.022 11.037l6.432 2.658c.545-.371 1.203-.59 1.912-.59.063 0 .125.004.188.006l2.861-4.142V8.91c0-2.495 2.028-4.524 4.524-4.524 2.494 0 4.524 2.031 4.524 4.527s-2.03 4.525-4.524 4.525h-.105l-4.076 2.911c0 .052.004.105.004.159 0 1.875-1.515 3.396-3.39 3.396-1.635 0-3.016-1.173-3.331-2.727L.436 15.27C1.862 20.307 6.486 24 11.979 24c6.627 0 11.999-5.373 11.999-12S18.605 0 11.979 0zM7.54 18.21l-1.473-.61c.262.543.714.999 1.314 1.25 1.297.539 2.793-.076 3.332-1.375.263-.63.264-1.319.005-1.949s-.75-1.121-1.377-1.383c-.624-.26-1.29-.249-1.878-.03l1.523.63c.956.4 1.409 1.5 1.009 2.455-.397.957-1.497 1.41-2.454 1.012H7.54zm11.415-9.303c0-1.662-1.353-3.015-3.015-3.015-1.665 0-3.015 1.353-3.015 3.015 0 1.665 1.35 3.015 3.015 3.015 1.663 0 3.015-1.35 3.015-3.015zm-5.273-.005c0-1.252 1.013-2.266 2.265-2.266 1.249 0 2.266 1.014 2.266 2.266 0 1.251-1.017 2.265-2.266 2.265-1.253 0-2.265-1.014-2.265-2.265z",
  warning: "M12 5.99 19.53 19H4.47L12 5.99M12 2 1 21h22L12 2zm1 14h-2v2h2v-2zm0-6h-2v4h2v-4z",
  openExternal: "M19 19H5V5h7V3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14c1.1 0 2-.9 2-2v-7h-2zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3z",
  bookmark: "M17 3H7a2 2 0 0 0-2 2v16l7-3 7 3V5a2 2 0 0 0-2-2zm0 15-5-2.18L7 18V5h10z",
  bookmarkOn: "M17 3H7a2 2 0 0 0-2 2v16l7-3 7 3V5a2 2 0 0 0-2-2z",
  bell: "M12 22c1.1 0 2-.9 2-2h-4c0 1.1.9 2 2 2zm6-6v-5c0-3.07-1.63-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.64 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2zm-2 1H8v-6c0-2.48 1.51-4.5 4-4.5s4 2.02 4 4.5v6z",
  stage: "M11 2h2v3.5h-2zM9 6h6v1.6H9zM4 8.6h16v2.2H4zM6.2 12h11.6l1.2 10H5zM8.6 14 7.9 20h8.2l-.7-6z",
};

const STROKE_ICONS = {
  hand: "M18 11V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2M14 10V4a2 2 0 0 0-2-2a2 2 0 0 0-2 2v2M10 10.5V6a2 2 0 0 0-2-2a2 2 0 0 0-2 2v8M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15",
  emojiSmile: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0M8 14s1.5 2 4 2 4-2 4-2M9 9h.01M15 9h.01",
  emojiClock: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0M12 6v6l4 2",
  emojiLeaf: "M11 20A7 7 0 0 1 9.8 6.1C15.5 5 17 4.48 19 2c1 2 2 4.18 2 8 0 5.5-4.78 10-10 10ZM2 21c0-3 1.85-5.36 5.08-6C9.5 14.52 12 13 13 12",
  emojiFood: "M12 20.94c1.5 0 2.75 1.06 4 1.06 3 0 6-8 6-12.22A4.91 4.91 0 0 0 17 5c-2.22 0-4 1.44-5 2-1-.56-2.78-2-5-2a4.9 4.9 0 0 0-5 4.78C2 14 5 22 8 22c1.25 0 2.5-1.06 4-1.06ZM10 2c1 .5 2 2 2 5",
  emojiCar: "M19 17h2c.6 0 1-.4 1-1v-3c0-.9-.7-1.7-1.5-1.9C18.7 10.6 16 10 16 10s-1.3-1.4-2.2-2.3c-.5-.4-1.1-.7-1.8-.7H5c-.6 0-1.1.4-1.4.9l-1.4 2.9A3.7 3.7 0 0 0 2 12v4c0 .6.4 1 1 1h2M9 17h6M5 17a2 2 0 1 0 4 0a2 2 0 1 0-4 0M15 17a2 2 0 1 0 4 0a2 2 0 1 0-4 0",
  emojiBall: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0M19.13 5.09C15.22 9.14 10 10.44 2.25 10.94M21.75 12.84c-6.62-1.41-12.14 1-16.38 6.32M8.56 2.75c4.37 6 6 9.42 8 17.72",
  emojiBulb: "M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5M9 18h6M10 22h4",
  emojiHeart: "M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7Z",
  emojiFlag: "M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1zM4 22v-7",
  globe: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20M2 12h20",
};

function svgIcon(name, size = 20) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", size);
  svg.setAttribute("height", size);
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS(ns, "path");
  if (STROKE_ICONS[name]) {
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", "currentColor");
    path.setAttribute("stroke-width", "2");
    path.setAttribute("stroke-linecap", "round");
    path.setAttribute("stroke-linejoin", "round");
    path.setAttribute("d", STROKE_ICONS[name]);
  } else {
    path.setAttribute("fill", "currentColor");
    path.setAttribute("d", ICONS[name]);
  }
  svg.appendChild(path);
  return svg;
}

function voiceKindIcon(kind, size = 18) {
  return svgIcon(kind === "stage" ? "stage" : "speaker", size);
}

function setIcon(button, name, size = 20) {
  button.replaceChildren(svgIcon(name, size));
}

function setVoiceStatus(node, name, title) {
  node.replaceChildren(svgIcon(name, 14));
  node.classList.add("off");
  node.title = title;
}

function openSettingsLayer(id, tab) {
  hideModals();
  for (const layer of document.querySelectorAll(".settings-layer")) layer.classList.add("hidden");
  $(id).classList.remove("hidden");
  selectSettingsTab(id, tab);
  for (const range of $(id).querySelectorAll('input[type="range"]')) syncRangeFill(range);
  if (isPhoneWidth(window.innerWidth)) history.pushState({ pane: phonePane(), settings: true }, "");
}

function syncRangeFill(range) {
  const min = Number(range.min || 0);
  const max = Number(range.max || 100);
  const pct = ((Number(range.value) - min) / (max - min)) * 100;
  range.style.setProperty("--fill", `${pct}%`);
}
document.addEventListener("input", e => {
  if (e.target.matches?.('input[type="range"]')) syncRangeFill(e.target);
});

function selectSettingsTab(id, tab) {
  const layer = $(id);
  for (const btn of layer.querySelectorAll(".settings-tab[data-tab]")) {
    btn.classList.toggle("active", btn.dataset.tab === tab);
    if (btn.dataset.tab === tab && isPhoneWidth(window.innerWidth)) btn.scrollIntoView({ block: "nearest", inline: "center" });
  }
  for (const page of layer.querySelectorAll(".settings-page")) {
    page.classList.toggle("hidden", page.dataset.page !== tab);
  }
  layer.querySelector(".settings-content").scrollTop = 0;
  layer.dataset.tab = tab;
  settingsPhoneView(layer, "page");
  if (tab === "voice") prepareAvatarStage();
}

function hideEmptyNavGroups(layer) {
  let group = null;
  let shown = false;
  const settle = () => group && group.classList.toggle("hidden", !shown);
  for (const item of layer.querySelector(".settings-nav-inner").children) {
    if (item.classList.contains("settings-nav-group") || item.classList.contains("settings-nav-sep")) {
      settle();
      group = item.classList.contains("settings-nav-group") ? item : null;
      shown = false;
    } else if (item.classList.contains("settings-tab") && !item.classList.contains("hidden")) {
      shown = true;
    }
  }
  settle();
}

function openSettingsLayerId() {
  const open = document.querySelector(".settings-layer:not(.hidden)");
  return open ? open.id : null;
}

async function closeSettingsLayers() {
  if (state.micTesting) await stopMicTest();
  stopMicMeter();
  stopCameraPreview();
  for (const layer of document.querySelectorAll(".settings-layer")) layer.classList.add("hidden");
}

const APPEARANCE_KEY = "commy.appearance";
const APPEARANCE_DEFAULTS = { chatSize: 16, compact: false };

function loadAppearance() {
  try {
    return { ...APPEARANCE_DEFAULTS, ...JSON.parse(localStorage.getItem(APPEARANCE_KEY) || "{}") };
  } catch {
    return { ...APPEARANCE_DEFAULTS };
  }
}

function applyAppearance(prefs) {
  document.documentElement.style.setProperty("--chat-font-size", `${prefs.chatSize}px`);
  document.body.classList.toggle("compact", Boolean(prefs.compact));
}

function saveAppearance(prefs) {
  applyAppearance(prefs);
  try { localStorage.setItem(APPEARANCE_KEY, JSON.stringify(prefs)); } catch {}
}

const OS_REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";
const OS_DARK_QUERY = "(prefers-color-scheme: dark)";

function osPrefers(query) {
  return typeof matchMedia === "function" && matchMedia(query).matches;
}

function loadAccessibility() {
  let raw = null;
  try { raw = localStorage.getItem(ACCESSIBILITY_KEY); } catch {}
  return readAccessibility(raw, availableLanguages().map(l => l.code));
}

function applyTranslations(root) {
  for (const node of root.querySelectorAll("[data-i18n]")) node.textContent = t(node.dataset.i18n);
  for (const node of root.querySelectorAll("[data-i18n-title]")) node.title = t(node.dataset.i18nTitle);
}

function renderAccessibilitySettings(prefs, resolved) {
  $("reduced-motion-toggle").checked = resolved.reducedMotion;
  $("reduced-motion-hint").classList.toggle("hidden", !resolved.followsOsMotion);
  $("saturation").value = resolved.saturation;
  $("saturation-label").textContent = `${resolved.saturation}%`;
  $("high-contrast-toggle").checked = resolved.highContrast;
  $("theme-select").value = prefs.theme;
  const languages = $("language-select");
  languages.replaceChildren(...availableLanguages().map(l => new Option(l.label, l.code)));
  languages.value = prefs.language;
}

function applyAccessibility(prefs) {
  const resolved = resolveAccessibility(prefs, { reducedMotion: osPrefers(OS_REDUCED_MOTION_QUERY), dark: osPrefers(OS_DARK_QUERY) });
  const html = document.documentElement;
  for (const [name, value] of Object.entries(accessibilityAttributes(resolved))) {
    if (value === null) html.removeAttribute(name);
    else html.setAttribute(name, value);
  }
  html.style.setProperty("--saturation", saturationCss(resolved));
  html.lang = setLanguage(resolved.language);
  applyTranslations(document);
  renderAccessibilitySettings(prefs, resolved);
  return resolved;
}

function saveAccessibility(prefs) {
  try { localStorage.setItem(ACCESSIBILITY_KEY, writeAccessibility(prefs)); } catch {}
  applyAccessibility(prefs);
}

function wireAccessibility() {
  const prefs = loadAccessibility();
  applyAccessibility(prefs);
  if (typeof matchMedia === "function") {
    for (const query of [OS_REDUCED_MOTION_QUERY, OS_DARK_QUERY]) {
      matchMedia(query).addEventListener("change", () => applyAccessibility(prefs));
    }
  }
  $("reduced-motion-toggle").addEventListener("change", e => {
    prefs.reducedMotion = e.target.checked;
    saveAccessibility(prefs);
  });
  $("saturation").addEventListener("input", e => {
    prefs.saturation = Number(e.target.value);
    saveAccessibility(prefs);
  });
  $("high-contrast-toggle").addEventListener("change", e => {
    prefs.highContrast = e.target.checked;
    saveAccessibility(prefs);
  });
  $("theme-select").addEventListener("change", e => {
    prefs.theme = e.target.value;
    saveAccessibility(prefs);
  });
  $("language-select").addEventListener("change", e => {
    prefs.language = e.target.value;
    saveAccessibility(prefs);
  });
}

function paintRole(node, color) {
  node.style.color = color ? `#${color}` : "";
  node.classList.toggle("role-ink", Boolean(color));
}

function toast(message, { avatar = null, onClick = null } = {}) {
  const node = $("toast");
  node.replaceChildren(...(avatar ? [avatar] : []), el("span", "toast-text", message));
  node.classList.toggle("toast-click", Boolean(onClick));
  node.onclick = onClick ? () => {
    node.classList.add("hidden");
    onClick();
  } : null;
  node.classList.remove("hidden");
  clearTimeout(node._timer);
  node._timer = setTimeout(() => node.classList.add("hidden"), onClick ? 6000 : 3000);
}

function showModal(id) {
  if (id !== "modal-stream-deck") answerStreamDeckPairing(false);
  $("modal-backdrop").classList.remove("hidden");
  for (const modal of document.querySelectorAll(".modal")) modal.classList.add("hidden");
  $(id).classList.remove("hidden");
}

function hideModals() {
  answerStreamDeckPairing(false);
  stopSoundPreview();
  $("modal-backdrop").classList.add("hidden");
}

const avatarDirectory = new Map();
const avatarImages = new Map();
const OWN_AVATAR_RETRY_MS = [1000, 2000, 4000, 8000, 15000, 30000];

function rememberAvatars(members) {
  for (const m of members || []) {
    avatarDirectory.set(m.id, { avatar: m.avatar || null, provider: m.avatar_provider || null });
  }
}

function avatarFor(userId) {
  if (state.user && userId === state.user.id) {
    return { avatar: state.user.avatar || null, provider: state.user.node || null };
  }
  return avatarDirectory.get(userId) || null;
}

function cachedLoad(cache, key, load) {
  if (!cache.has(key)) {
    const pending = load()
      .then(url => {
        cache.set(key, url);
        return url;
      })
      .catch(() => {
        cache.delete(key);
        return null;
      });
    cache.set(key, pending);
  }
  return Promise.resolve(cache.get(key));
}

function loadAvatarImage(blob, provider) {
  return cachedLoad(avatarImages, blob, () => invoke("get_profile_image", { blob, provider }));
}

function loadAvatarImageUntil(blob, provider, waits, wanted) {
  return loadAvatarImage(blob, provider).then(url => {
    if (url || !waits.length || !wanted()) return url;
    return new Promise(resolve => setTimeout(resolve, waits[0]))
      .then(() => wanted() ? loadAvatarImageUntil(blob, provider, waits.slice(1), wanted) : null);
  });
}

function paintAvatar(node, userId, name) {
  for (const child of [...node.childNodes]) {
    if (child.nodeType === Node.TEXT_NODE || child.classList?.contains("avatar-face")) child.remove();
  }
  node.classList.remove("emoji-avatar", "image-avatar");
  node.style.setProperty("--tint", personTint(userId));
  const raw = avatarFor(userId)?.avatar || "";
  let face;
  if (raw.startsWith("emoji:")) {
    face = el("span", "avatar-face", raw.slice(6));
    node.classList.add("emoji-avatar");
  } else if (raw.startsWith("blob:")) {
    const blob = raw.slice(5);
    const ready = avatarImages.get(blob);
    face = el("span", "avatar-face", initials(name || "?"));
    const show = url => {
      if (!url || !node.contains(face)) return;
      const img = el("img", "avatar-face");
      img.src = url;
      img.alt = "";
      img.draggable = false;
      face.replaceWith(img);
      node.classList.add("image-avatar");
    };
    if (typeof ready === "string") {
      face = el("img", "avatar-face");
      face.src = ready;
      face.alt = "";
      face.draggable = false;
      node.classList.add("image-avatar");
    } else {
      const waits = state.user && userId === state.user.id ? OWN_AVATAR_RETRY_MS : [];
      loadAvatarImageUntil(blob, avatarFor(userId)?.provider, waits, () => node.isConnected && node.contains(face)).then(show);
    }
  } else {
    face = el("span", "avatar-face", initials(name || "?"));
  }
  node.prepend(face);
  return node;
}

function liveElapsed(sinceSecs, fmt = "clock", cls = "") {
  const node = el("span", "elapsed" + (cls ? ` ${cls}` : ""));
  node.dataset.since = String(sinceSecs || 0);
  node.dataset.fmt = fmt;
  node.textContent = fmt === "words" ? elapsedWords(sinceSecs) : elapsedClock(sinceSecs);
  return node;
}

setInterval(() => {
  for (const node of document.querySelectorAll(".elapsed[data-since]")) {
    const since = Number(node.dataset.since);
    const text = node.dataset.fmt === "words" ? elapsedWords(since) : elapsedClock(since);
    if (node.textContent !== text) node.textContent = text;
  }
}, 1000);

function activityWithTime(person) {
  if (!person || !person.activity) return "";
  return person.activity_since ? `${person.activity} for ${elapsedWords(person.activity_since)}` : person.activity;
}

function voiceChannelOf(member, server) {
  return member.voice_channel && server && (server.channels || []).find(c => c.id === member.voice_channel);
}

function memberHover(member, channel) {
  const lines = [];
  if (channel) lines.push(`In voice · ${channelPlainName(channel)} · ${elapsedClock(member.voice_since)}`);
  const game = activityWithTime(member);
  if (game) lines.push(game);
  return lines.join("\n");
}

function makeAvatar(userId, name, className = "avatar") {
  return paintAvatar(el("div", className), userId, name);
}

function initials(name) {
  return name.split(/\s+/).map(w => w[0]).join("").slice(0, 2).toUpperCase();
}

function formatTime(iso) {
  const date = new Date(iso);
  const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const day = dayLabel(date);
  return day === "Today" ? time : `${day} at ${time}`;
}

function dayLabel(date) {
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (date.toDateString() === today.toDateString()) return "Today";
  if (date.toDateString() === yesterday.toDateString()) return "Yesterday";
  const sameYear = date.getFullYear() === today.getFullYear();
  return date.toLocaleDateString([], sameYear ? { month: "long", day: "numeric" } : { year: "numeric", month: "long", day: "numeric" });
}

document.addEventListener("contextmenu", e => {
  const editable = e.target.closest && e.target.closest("input, textarea, [contenteditable]:not([contenteditable=\"false\"])");
  const selected = window.getSelection && String(window.getSelection()).length > 0;
  if (!editable && !selected) e.preventDefault();
});

const RAIL_KEY = "commy.railLayout";

function loadRailLayout() {
  try {
    const parsed = JSON.parse(localStorage.getItem(RAIL_KEY) || "[]");
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function saveRailLayout(layout) {
  try { localStorage.setItem(RAIL_KEY, JSON.stringify(layout)); } catch {}
}

function normalizeRailLayout(layout, serverIds) {
  const known = new Set(serverIds);
  const seen = new Set();
  const keep = id => known.has(id) && !seen.has(id) && seen.add(id);
  const out = [];
  for (const item of layout) {
    if (item.type === "server" && keep(item.id)) out.push({ type: "server", id: item.id });
    if (item.type === "folder") {
      const servers = (item.servers || []).filter(keep);
      if (servers.length > 1) out.push({ ...item, servers });
      else if (servers.length === 1) out.push({ type: "server", id: servers[0] });
    }
  }
  for (const id of serverIds) if (!seen.has(id)) out.push({ type: "server", id });
  return out;
}

function removeFromLayout(layout, id) {
  return layout
    .filter(item => !(item.type === "server" && item.id === id))
    .map(item => (item.type === "folder" ? { ...item, servers: item.servers.filter(s => s !== id) } : item));
}

function applyRailDrop(layout, drag, target, zone) {
  if (drag.id === target.id) return layout;
  if (drag.kind === "folder") {
    if (target.folder) return layout;
    const folder = layout.find(i => i.type === "folder" && i.id === drag.id);
    let rest = layout.filter(i => i !== folder);
    const at = rest.findIndex(i => i.id === target.id);
    if (!folder || at < 0) return layout;
    rest.splice(zone === "after" ? at + 1 : at, 0, folder);
    return rest;
  }
  let next = removeFromLayout(layout, drag.id);
  if (target.folder) {
    next = next.map(item => {
      if (item.type !== "folder" || item.id !== target.folder) return item;
      const servers = item.servers.slice();
      const at = servers.indexOf(target.id);
      servers.splice(zone === "after" ? at + 1 : Math.max(at, 0), 0, drag.id);
      return { ...item, servers };
    });
  } else if (zone === "into") {
    const at = next.findIndex(i => i.id === target.id);
    if (at < 0) return layout;
    const hit = next[at];
    next[at] = hit.type === "folder"
      ? { ...hit, servers: [...hit.servers, drag.id] }
      : { type: "folder", id: `f${Date.now().toString(36)}`, name: "", open: false, servers: [hit.id, drag.id] };
  } else {
    const at = next.findIndex(i => i.id === target.id);
    if (at < 0) return layout;
    next.splice(zone === "after" ? at + 1 : at, 0, { type: "server", id: drag.id });
  }
  return normalizeRailLayout(next, railServers().map(s => s.id));
}

let railDrag = null;

function dropZone(e, node, allowInto) {
  const r = node.getBoundingClientRect();
  const x = (e.clientX - r.left) / r.width;
  if (!allowInto) return x < 0.5 ? "before" : "after";
  return x < 0.25 ? "before" : x > 0.75 ? "after" : "into";
}

function makeRailDropTarget(node, target) {
  node.addEventListener("dragover", e => {
    if (!railDrag || railDrag.id === target.id) return;
    e.preventDefault();
    const zone = dropZone(e, node, railDrag.kind === "server");
    node.dataset.drop = zone;
  });
  node.addEventListener("dragleave", () => delete node.dataset.drop);
  node.addEventListener("drop", e => {
    e.preventDefault();
    const zone = node.dataset.drop;
    delete node.dataset.drop;
    if (!railDrag || !zone) return;
    const layout = applyRailDrop(normalizeRailLayout(loadRailLayout(), railServers().map(s => s.id)), railDrag, target, zone);
    saveRailLayout(layout);
    railDrag = null;
    renderServerRail();
  });
}

function shownUnread(serverId, channelId) {
  const count = unread.channels.get(channelId) || 0;
  if (!count) return 0;
  if (!unread.mentions.has(channelId) && isChannelMuted(serverId, channelId)) return 0;
  return count;
}

function serverUnreadCount(serverId) {
  if (isServerMuted(serverId)) return 0;
  let total = 0;
  for (const [channel, server] of unread.channelServer) {
    if (server === serverId) total += shownUnread(serverId, channel);
  }
  return total;
}

function countBadge(count) {
  const badge = el("span", "count-badge", count > 99 ? "99+" : String(count));
  return badge;
}

function railServers() {
  return state.servers.filter(s => !s.group);
}

function isGroupServer(serverId) {
  return state.servers.some(s => s.id === serverId && s.group);
}

function railServerButton(server, folderId) {
  const btn = el("button", "server-icon");
  paintServerIcon(btn, server);
  btn.title = server.name;
  btn.draggable = true;
  btn.dataset.id = server.id;
  if (state.currentServer && server.id === state.currentServer.id) btn.classList.add("active");
  const count = serverUnreadCount(server.id);
  btn.classList.toggle("unread", count > 0);
  btn.classList.toggle("muted", isServerMuted(server.id));
  const wrap = el("div", "rail-item");
  wrap.appendChild(btn);
  if (count && !isServerMuted(server.id)) wrap.appendChild(countBadge(count));
  btn.onclick = () => selectServer(server.id);
  btn.oncontextmenu = e => {
    e.preventDefault();
    openServerMenu(server, e.clientX, e.clientY, folderId);
  };
  btn.addEventListener("dragstart", e => {
    railDrag = { kind: "server", id: server.id };
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", server.id);
  });
  btn.addEventListener("dragend", () => (railDrag = null));
  makeRailDropTarget(btn, { kind: "server", id: server.id, folder: folderId || null });
  return wrap;
}

function plainServer() {
  return state.view === "server" && state.currentServer && !state.currentServer.group ? state.currentServer : null;
}

function paintTitlebar() {
  if (!document.body.classList.contains("has-titlebar")) return;
  const server = plainServer();
  const icon = $("titlebar-icon");
  $("titlebar-name").textContent = server ? server.name : "Commy";
  if (server) {
    paintServerIcon(icon, server);
  } else {
    icon.classList.remove("has-image");
    const img = el("img");
    img.src = "images/commy_v3_noback.png";
    img.alt = "";
    img.draggable = false;
    icon.replaceChildren(img);
  }
}

async function setupTitlebar() {
  let custom = false;
  try { custom = IS_TAURI && await invoke("has_custom_titlebar"); } catch {}
  if (!custom) return;
  document.body.classList.add("has-titlebar");
  $("titlebar").classList.remove("hidden");
  const act = action => invoke("window_action", { action }).catch(() => {});
  $("tb-min").addEventListener("click", () => act("minimize"));
  $("tb-max").addEventListener("click", () => act("toggle-maximize"));
  $("tb-close").addEventListener("click", () => act("close"));
  $("titlebar-drag").addEventListener("mousedown", e => { if (e.button === 0 && e.detail === 1) act("drag"); });
  $("titlebar-drag").addEventListener("dblclick", () => act("toggle-maximize"));
  paintTitlebar();
}

function syncWindowTitle() {
  paintTitlebar();
  const server = plainServer();
  const name = server ? server.name : null;
  if (name === state.windowTitleServer) return;
  state.windowTitleServer = name;
  document.title = name ? `${name} - Commy` : "Commy";
  invoke("set_window_title", { server: name }).catch(() => {});
}

function renderServerRail() {
  syncWindowTitle();
  const list = $("server-list");
  list.innerHTML = "";
  const dmCount = [...unread.dms.values()].reduce((a, b) => a + b, 0)
    + state.servers.filter(s => s.group).reduce((n, s) => n + serverUnreadCount(s.id), 0);
  $("home-btn").classList.toggle("unread", dmCount > 0);
  $("home-badge")?.remove();
  if (dmCount) {
    const badge = countBadge(dmCount);
    badge.id = "home-badge";
    $("home-btn").appendChild(badge);
  }
  const byId = new Map(railServers().map(s => [s.id, s]));
  const layout = normalizeRailLayout(loadRailLayout(), railServers().map(s => s.id));
  saveRailLayout(layout);
  for (const item of layout) {
    if (item.type === "server") {
      list.appendChild(railServerButton(byId.get(item.id)));
      continue;
    }
    const servers = item.servers.map(id => byId.get(id)).filter(Boolean);
    const folder = el("div", "rail-folder" + (item.open ? " open" : ""));
    const head = el("button", "folder-head");
    head.title = item.name || servers.map(s => s.name).join(", ");
    head.draggable = true;
    if (item.open) {
      head.appendChild(svgIcon("folder", 22));
    } else {
      const grid = el("div", "folder-grid");
      for (const s of servers.slice(0, 4)) {
        const mini = el("span", "folder-mini");
        paintServerIcon(mini, s);
        mini.style.background = serverTint(s.id);
        grid.appendChild(mini);
      }
      head.appendChild(grid);
      const count = servers.reduce((n, s) => n + serverUnreadCount(s.id), 0);
      if (count) folder.appendChild(countBadge(count));
      if (servers.some(s => state.currentServer && s.id === state.currentServer.id)) head.classList.add("active");
    }
    head.onclick = () => {
      const fresh = loadRailLayout().map(i => (i.id === item.id ? { ...i, open: !i.open } : i));
      saveRailLayout(fresh);
      renderServerRail();
    };
    head.oncontextmenu = e => {
      e.preventDefault();
      openFolderMenu(item, e.clientX, e.clientY);
    };
    head.addEventListener("dragstart", e => {
      railDrag = { kind: "folder", id: item.id };
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", item.id);
    });
    head.addEventListener("dragend", () => (railDrag = null));
    makeRailDropTarget(head, { kind: "folder", id: item.id, folder: null });
    folder.prepend(head);
    if (item.open) for (const s of servers) folder.appendChild(railServerButton(s, item.id));
    list.appendChild(folder);
  }
  paintNarrowRail();
}

let menuServer = null;
let menuFolderOf = null;
let menuFolder = null;

function serverPrefs(serverId) {
  const prefs = notifyPrefs();
  return { level: prefs.servers[serverId] || "default", muted: Boolean(prefs.muted && prefs.muted[serverId]) };
}

function isServerMuted(serverId) {
  return serverPrefs(serverId).muted;
}

function nowSecs() {
  return Math.floor(Date.now() / 1000);
}

function channelMute(serverId, channelId) {
  const own = channelPref(notifyPrefs(), serverId, channelId);
  if (!own) return null;
  if (own.mute === true) return true;
  return typeof own.mute === "number" && own.mute > nowSecs() ? own.mute : null;
}

function isChannelMuted(serverId, channelId) {
  return isMuted(notifyPrefs(), serverId, channelId, nowSecs());
}

function updateChannelPrefs(serverId, channelId, change) {
  saveNotifyPrefs(withChannelPref(notifyPrefs(), serverId, channelId, change));
  syncUnreadBadge();
  if (state.currentServer && state.currentServer.id === serverId) renderChannels();
}

function muteWords(mute) {
  if (mute === true) return "Muted until you turn it back on";
  const at = new Date(mute * 1000);
  const today = at.toDateString() === new Date().toDateString();
  const time = at.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  return today ? `Muted until ${time}` : `Muted until ${at.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}

let menuChannel = null;

function openChannelMenu(channel, x, y) {
  closeFloating();
  const server = state.currentServer;
  if (!server) return;
  menuChannel = channel;
  const own = channelPref(notifyPrefs(), server.id, channel.id) || {};
  const level = own.level || "default";
  const mute = channelMute(server.id, channel.id);
  $("channel-menu-title").replaceChildren(document.createTextNode("# "), channelNameNode(channel));
  for (const btn of $("channel-menu").querySelectorAll(".ctx-radio[data-level]")) {
    btn.classList.toggle("checked", btn.dataset.level === level);
  }
  for (const btn of $("channel-menu").querySelectorAll(".ctx-radio[data-mute]")) btn.classList.toggle("hidden", mute !== null);
  $("chm-unmute").classList.toggle("hidden", mute === null);
  $("chm-unmute").textContent = mute === null ? "" : muteWords(mute);
  $("chm-pin").textContent = channelIsPinned(server.id, channel.id) ? "Unpin Channel" : "Pin Channel";
  $("chm-settings").classList.toggle("hidden", !server.can_manage_channels);
  showVoiceStatusMenuItem(channel, server);
  showVoiceCallMenuItems(channel);
  placeMenuAt($("channel-menu"), x, y);
}

function showVoiceStatusMenuItem(channel, server) {
  const allowed = canSetVoiceStatus({ channel, server, me: state.user && state.user.id });
  const item = $("chm-voice-status");
  item.classList.toggle("hidden", !allowed);
  item.textContent = channel.voice_status ? "Change Voice Status" : "Set Voice Status";
  $("chm-read").classList.toggle("hidden", channel.kind === "voice");
}

let voiceStatusChannel = null;

function openVoiceStatusModal(channel) {
  if (!channel || !state.currentServer) return;
  voiceStatusChannel = { serverId: state.currentServer.id, channelId: channel.id };
  $("voice-status-title").replaceChildren(svgIcon("speaker", 18), channelNameNode(channel));
  $("voice-status-input").value = channel.voice_status || "";
  showModal("modal-voice-status");
  $("voice-status-input").focus();
  $("voice-status-input").select();
}

async function saveVoiceStatus(text) {
  const target = voiceStatusChannel;
  if (!target) return;
  const problem = voiceStatusProblem(text);
  if (problem) {
    toast(problem);
    return;
  }
  try {
    await invoke("set_voice_status", { serverId: target.serverId, channelId: target.channelId, text: cleanVoiceStatus(text) });
    hideModals();
    voiceStatusChannel = null;
    if (state.currentServer && state.currentServer.id === target.serverId) await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

function wireVoiceStatus() {
  $("chm-voice-status").onclick = () => {
    const channel = menuChannel;
    closeFloating();
    openVoiceStatusModal(channel);
  };
  $("voice-status-save").onclick = () => saveVoiceStatus($("voice-status-input").value);
  $("voice-status-clear").onclick = () => saveVoiceStatus("");
  $("voice-status-cancel").onclick = hideModals;
  $("voice-status-input").addEventListener("keydown", e => {
    if (e.key === "Enter") saveVoiceStatus($("voice-status-input").value);
  });
  $("vp-topic").onclick = () => {
    const server = state.currentServer;
    const channel = server && state.voiceServer && server.id === state.voiceServer.id
      ? server.channels.find(c => c.id === state.voiceChannelId)
      : null;
    if (channel && canSetVoiceStatus({ channel, server, me: state.user && state.user.id })) openVoiceStatusModal(channel);
  };
  $("vp-conn-meta").oncontextmenu = e => {
    const server = state.currentServer;
    const channel = server && state.voiceServer && server.id === state.voiceServer.id
      ? server.channels.find(c => c.id === state.voiceChannelId)
      : null;
    if (!channel) return;
    e.preventDefault();
    openChannelMenu(channel, e.clientX, e.clientY);
  };
}

function voiceStatusLine(channel) {
  const line = el("div", "voice-status-line", channel.voice_status);
  line.title = channel.voice_status;
  return line;
}

function decorateVoiceUser(node, channel, userId) {
  const server = state.currentServer;
  if (!server || !userId) return;
  const member = (server.members || []).find(m => m.id === userId);
  if (member && member.priority_speaker) {
    const badge = el("span", "vs-icon priority");
    badge.appendChild(svgIcon("megaphone", 14));
    badge.title = "Priority speaker";
    node.appendChild(badge);
  }
  node.classList.toggle("attenuated", attenuatedUsers(channel, server.members, state.speaking).has(userId));
}

function paintAttenuation() {
  const server = state.currentServer;
  const channel = server && state.voiceChannelId ? server.channels.find(c => c.id === state.voiceChannelId) : null;
  const ducked = channel ? attenuatedUsers(channel, server.members, state.speaking) : new Set();
  for (const node of document.querySelectorAll(".voice-user[data-user]")) {
    node.classList.toggle("attenuated", ducked.has(node.dataset.user));
  }
}

function renderVoiceTopic(vs) {
  const topic = $("vp-topic");
  const server = state.currentServer && vs && state.currentServer.id === vs.id ? state.currentServer : null;
  const channel = server ? server.channels.find(c => c.id === state.voiceChannelId) : null;
  const text = channel && channel.voice_status ? channel.voice_status : "";
  topic.textContent = text;
  topic.classList.toggle("hidden", !text);
  const editable = Boolean(channel) && canSetVoiceStatus({ channel, server, me: state.user && state.user.id });
  topic.classList.toggle("editable", editable);
  topic.title = editable ? "Change the channel status" : text;
}

function placeMenuAt(menu, x, y) {
  menu.classList.remove("hidden");
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(x, window.innerWidth - r.width - 8)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - r.height - 8)}px`;
}

function openServerMenu(server, x, y, folderId) {
  closeFloating();
  menuServer = server;
  menuFolderOf = folderId || null;
  const { level, muted } = serverPrefs(server.id);
  $("server-menu-title").textContent = server.name;
  for (const btn of $("server-menu").querySelectorAll(".ctx-radio")) {
    btn.classList.toggle("checked", btn.dataset.level === level);
  }
  $("sm-mute").classList.toggle("checked", muted);
  $("sm-mute").textContent = muted ? "Unmute Server" : "Mute Server";
  $("sm-ungroup").classList.toggle("hidden", !menuFolderOf);
  $("sm-rules").classList.add("hidden");
  $("sm-roles").classList.add("hidden");
  $("sm-onboarding").classList.add("hidden");
  $("sm-settings").textContent = "Server Settings";
  $("sm-invite").classList.add("hidden");
  invoke("get_server", { serverId: server.id }).then(full => {
    $("sm-invite").classList.toggle("hidden", !full.can_invite);
    $("sm-rules").classList.toggle("hidden", !full.rules);
    $("sm-roles").classList.toggle("hidden", !selfAssignableRoles(full).length);
    $("sm-onboarding").classList.toggle("hidden", !full.onboarding);
    $("sm-settings").textContent = canOpenServerSettings(full) ? "Server Settings" : "Server Info";
  }).catch(() => {});
  placeMenuAt($("server-menu"), x, y);
}

function openFolderMenu(folder, x, y) {
  closeFloating();
  menuFolder = folder;
  placeMenuAt($("folder-menu"), x, y);
}

function updateServerPrefs(serverId, change) {
  const prefs = notifyPrefs();
  prefs.muted = prefs.muted || {};
  if ("level" in change) {
    if (change.level === "default") delete prefs.servers[serverId];
    else prefs.servers[serverId] = change.level;
  }
  if ("muted" in change) {
    if (change.muted) prefs.muted[serverId] = true;
    else delete prefs.muted[serverId];
  }
  saveNotifyPrefs(prefs);
  syncUnreadBadge();
}

const LEVEL_WORDS = { all: "All messages", mentions: "Only @mentions", online: "Only while I am online", none: "Nothing" };

let notifyServer = null;

function renderServerNotify() {
  if (!notifyServer) return;
  const prefs = notifyPrefs();
  const { level, muted } = serverPrefs(notifyServer.id);
  $("server-notify-name").textContent = notifyServer.name;
  $("server-notify-mute").checked = muted;
  $("server-notify-default").textContent = `Your setting for every server: ${LEVEL_WORDS[prefs.level] || LEVEL_WORDS.all}`;
  for (const btn of $("server-notify-levels").querySelectorAll(".notify-level")) {
    const picked = btn.dataset.level === level;
    btn.classList.toggle("picked", picked);
    btn.setAttribute("aria-checked", String(picked));
    btn.disabled = muted;
  }
  $("server-notify-everyone").checked = !filterOn(prefs, "suppressEveryone", notifyServer.id);
  $("server-notify-roles").checked = !filterOn(prefs, "suppressRoles", notifyServer.id);
  $("modal-server-notify").classList.toggle("muted", muted);
}

function openServerNotify(server) {
  notifyServer = server;
  renderServerNotify();
  showModal("modal-server-notify");
}

function setServerFilter(key, on) {
  if (!notifyServer) return;
  saveNotifyPrefs(withServerFilter(notifyPrefs(), notifyServer.id, key, on));
  syncUnreadBadge();
}

function markServerRead(serverId) {
  for (const [channel, server] of [...unread.channelServer]) {
    if (server === serverId) {
      unread.channels.delete(channel);
      unread.mentions.delete(channel);
    }
  }
  syncUnreadBadge();
  if (state.currentServer && state.currentServer.id === serverId) renderChannels();
  markChannelsRead(serverId, null);
}

let leavingServerId = null;

function openLeaveServer(serverId) {
  const server = state.servers.find(s => s.id === serverId);
  if (!server) return;
  leavingServerId = serverId;
  $("leave-server-title").textContent = server.group ? "Leave this group?" : `Leave '${server.name}'`;
  $("leave-server-body").textContent = server.group
    ? "You stop seeing its messages and calls, and your copy of the conversation is removed from this device."
    : "Are you sure you want to leave? You will not be able to rejoin unless you are invited again, and your copy of its history is removed from this device.";
  $("leave-server-go").textContent = server.group ? "Leave Group" : "Leave Server";
  showModal("modal-leave-server");
}

let deletingServerId = null;

function openTransferOwner(server) {
  const pick = $("transfer-owner-pick");
  pick.replaceChildren();
  const others = server.members.filter(m => !(state.user && m.id === state.user.id) && m.id !== server.owner && !m.bot);
  for (const member of others) {
    const option = el("option", null, member.name);
    option.value = member.id;
    pick.appendChild(option);
  }
  if (!others.length) {
    toast("There is nobody else here to hand the server to");
    return;
  }
  const input = $("transfer-owner-input");
  input.value = "";
  input.placeholder = server.name;
  const go = $("transfer-owner-go");
  go.disabled = true;
  input.oninput = () => {
    go.disabled = input.value.trim() !== server.name.trim();
  };
  showModal("modal-transfer-owner");
  input.focus();
}

async function confirmTransferOwner() {
  const server = state.currentServer;
  if (!server) return;
  const pick = $("transfer-owner-pick");
  const name = pick.selectedOptions[0] ? pick.selectedOptions[0].textContent : "them";
  try {
    await invoke("transfer_ownership", { serverId: server.id, userId: pick.value, confirmName: $("transfer-owner-input").value });
    hideModals();
    await refreshCurrentServer();
    if (!$("server-settings").classList.contains("hidden")) openServerSettings("overview");
    toast(`${name} now owns ${server.name}`);
  } catch (e) {
    toast(String(e));
  }
}

function openDeleteServer(server) {
  deletingServerId = server.id;
  $("delete-server-title").textContent = `Delete '${server.name}'`;
  const input = $("delete-server-input");
  input.value = "";
  input.placeholder = server.name;
  $("delete-server-go").disabled = true;
  input.oninput = () => {
    $("delete-server-go").disabled = input.value.trim() !== server.name.trim();
  };
  showModal("modal-delete-server");
  input.focus();
}

async function confirmDeleteServer() {
  const serverId = deletingServerId;
  if (!serverId) return;
  try {
    await invoke("delete_server", { serverId, confirmName: $("delete-server-input").value });
    deletingServerId = null;
    hideModals();
    await leaveServerView(serverId);
    toast("Server deleted");
  } catch (e) {
    toast(String(e));
  }
}

async function leaveServerView(serverId) {
  markServerRead(serverId);
  if (state.currentServer && state.currentServer.id === serverId) {
    await closeSettingsLayers();
    state.currentServer = null;
  }
  if (state.voiceServer && state.voiceServer.id === serverId) {
    state.voiceChannelId = null;
    state.voiceServer = null;
    state.sharing = false;
  }
  await refreshServers();
  if (!state.currentServer) {
    if (railServers().length) await selectServer(railServers()[0].id);
    else await enterDmMode();
  }
}

async function confirmLeaveServer() {
  const serverId = leavingServerId;
  if (!serverId) return;
  try {
    await invoke("leave_server", { serverId });
    hideModals();
    await closeSettingsLayers();
    markServerRead(serverId);
    if (state.currentServer && state.currentServer.id === serverId) state.currentServer = null;
    await refreshServers();
    if (!state.currentServer) {
      if (railServers().length) await selectServer(railServers()[0].id);
      else await enterDmMode();
    }
  } catch (e) {
    toast(String(e));
  }
}

const CHANNEL_EMOJI_RE = /:([a-z0-9_]{2,32}):/g;

function serverEmoji(name) {
  const server = state.currentServer;
  return ((server && server.emojis) || []).find(e => e.name === name) || null;
}

function channelNameParts(name) {
  const parts = [];
  const lead = splitLeadingEmoji(name);
  if (lead.lead) parts.push({ glyph: lead.lead, separator: lead.separator });
  let last = lead.lead ? lead.lead.length + lead.separator.length : 0;
  CHANNEL_EMOJI_RE.lastIndex = last;
  let m;
  while ((m = CHANNEL_EMOJI_RE.exec(name))) {
    const emoji = serverEmoji(m[1]);
    if (!emoji) continue;
    let text = name.slice(last, m.index);
    if (parts.length && typeof parts[parts.length - 1] === "object") text = text.replace(/^-/, "");
    text = text.replace(/-$/, "");
    if (text) parts.push(text);
    parts.push({ emoji });
    last = m.index + m[0].length;
  }
  let tail = name.slice(last);
  if (parts.length && typeof parts[parts.length - 1] === "object") tail = tail.replace(/^-/, "");
  if (tail) parts.push(tail);
  return parts;
}

function paintChannelBand(row, channel) {
  if (channel.banner) {
    row.classList.add("has-banner");
    loadServerImage(channel.banner).then(url => {
      if (url) row.style.setProperty("--channel-banner", `url("${url}")`);
    });
  } else if (channel.color) {
    row.classList.add("has-band");
    row.style.setProperty("--channel-band", roleGradient(channel.color));
  }
}

function channelNameNode(channel, cls = null) {
  const span = el("span", cls);
  for (const part of channelNameParts(channel.name)) {
    if (typeof part === "string") span.appendChild(document.createTextNode(part));
    else if (part.glyph) span.appendChild(el("span", "channel-emoji-glyph", part.glyph));
    else span.appendChild(customEmojiImg(part.emoji.name, part.emoji.image, "custom-emoji channel-emoji"));
  }
  return span;
}

function channelPlainName(channel) {
  const parts = channelNameParts(channel.name);
  const lead = parts[0] && parts[0].glyph ? parts.shift() : null;
  const text = parts.filter(p => typeof p === "string").join(" ").trim();
  return (lead ? `${lead.glyph}${lead.separator}${text}` : text) || channel.name;
}

function renderChannels() {
  const list = $("channel-list");
  list.innerHTML = "";
  if (!state.currentServer) return;
  const server = state.currentServer;
  checkVoiceRoster();
  reconcileActivity();
  if (server.group) {
    renderDmThreads();
    return;
  }
  $("server-name").textContent = server.name;
  paintNarrowRail();
  paintSidebarBanner();
  renderServerExtras(list, server);

  const canCreate = Boolean(server.can_manage_channels);
  const collapsed = collapsedSections();
  for (const section of pinnedFirst(channelSections(server, canCreate), server, canCreate)) {
    const isCollapsed = collapsed.has(section.key);
    list.appendChild(sectionHeader(section, isCollapsed, canCreate && !section.fixed));
    for (const channel of section.channels) {
      const active = (state.currentChannel && channel.id === state.currentChannel.id) || channel.id === state.voiceChannelId;
      if (isCollapsed && !active) continue;
      const row = isVoiceKind(channel.kind) ? voiceChannelRow(channel, canCreate, section.pinned) : textChannelRow(channel, canCreate, section.pinned);
      if (canCreate && !section.fixed) {
        row.draggable = true;
        row.addEventListener("dragstart", e => {
          dragChannel = channel;
          e.dataTransfer.effectAllowed = "move";
        });
        makeDropTarget(row, () => dropChannel(section, channel.id));
      }
      list.appendChild(row);
      if (channel.kind === "voice" && channel.voice_status) list.appendChild(voiceStatusLine(channel));
      if (isVoiceKind(channel.kind)) appendVoiceUsers(list, channel);
    }
  }
  scheduleMuteExpiry(server);
  pushOverlayRoster();
}

function pinnedFirst(sections, server, canCreate) {
  const ids = pinnedChannelIds(listPins, server.id);
  if (!ids.length) return sections;
  const pinned = [];
  const kept = [];
  for (const section of sections) {
    const split = splitPinned(section.channels, ids, c => c.id);
    pinned.push(...split.pinned);
    if (split.pinned.length && !split.rest.length && !canCreate) continue;
    kept.push({ ...section, channels: split.rest });
  }
  if (!pinned.length) return sections;
  kept.unshift({ key: `${server.id}:pinned`, title: "Pinned", kind: "text", parent: null, fixed: true, pinned: true, channels: pinned });
  return kept;
}

let muteExpiryTimer = null;

function scheduleMuteExpiry(server) {
  clearTimeout(muteExpiryTimer);
  muteExpiryTimer = null;
  const now = nowSecs();
  const soonest = server.channels
    .map(c => channelMute(server.id, c.id))
    .filter(mute => typeof mute === "number" && mute > now)
    .reduce((a, b) => Math.min(a, b), Infinity);
  if (!Number.isFinite(soonest)) return;
  muteExpiryTimer = setTimeout(() => {
    if (state.currentServer && state.currentServer.id === server.id) renderChannels();
    syncUnreadBadge();
  }, Math.min((soonest - now) * 1000 + 500, 2_147_000_000));
}

function channelMark(channel, size = 18) {
  if (channel && channel.kind === "announcement") {
    const icon = el("span", "channel-icon announce-icon");
    icon.appendChild(svgIcon("megaphone", size));
    return icon;
  }
  if (isForumChannel(channel)) {
    const icon = el("span", "channel-icon forum-icon");
    icon.appendChild(svgIcon("forum", size));
    return icon;
  }
  return el("span", "hash", "#");
}

function channelHeaderMark(channel) {
  if (channel && channel.kind === "announcement") return svgIcon("megaphone", 22);
  if (isForumChannel(channel)) return svgIcon("forum", 22);
  return document.createTextNode("#");
}

function channelKindMark(kind, size = 16) {
  if (kind === "voice") return svgIcon("speaker", size);
  if (kind === "stage") return svgIcon("stage", size);
  if (kind === "category") return svgIcon("chevronDown", size);
  if (kind === "announcement") return svgIcon("megaphone", size);
  if (kind === "forum") return svgIcon("forum", size);
  return document.createTextNode("#");
}

function kindHoldsMessages(kind) {
  return kind === "text" || kind === "announcement" || kind === "forum";
}

function channelLockMark(row, channel) {
  if (!channelIsPrivate(channel)) return;
  const mark = el("span", "channel-lock");
  mark.appendChild(svgIcon("lock", 14));
  mark.title = channel.can_view === false
    ? `Only ${accessWho(state.currentServer, channel.view_roles)} can read this channel`
    : `Only ${accessWho(state.currentServer, channel.view_roles)} can see this channel`;
  row.appendChild(mark);
  if (channel.can_view === false) row.classList.add("unreadable");
}

function channelBellSlot(row, channel) {
  const slot = el("span", "channel-bell");
  const serverId = state.currentServer && state.currentServer.id;
  if (serverId && channel.kind !== "voice" && isChannelMuted(serverId, channel.id) && !isServerMuted(serverId)) {
    slot.appendChild(svgIcon("bellOff", 14));
    slot.title = muteWords(channelMute(serverId, channel.id));
    row.classList.add("muted");
  }
  row.appendChild(slot);
}

function channelSensitiveMark(row, channel) {
  if (!showsSensitiveBadge(channel)) return;
  const mark = el("span", "channel-age", "18+");
  mark.title = "Age-restricted channel";
  row.appendChild(mark);
}

function textChannelRow(channel, canCreate, pinned = false) {
  const row = el("div", "channel");
  row.appendChild(channelMark(channel));
  row.appendChild(channelNameNode(channel, "channel-label"));
  channelLockMark(row, channel);
  channelSensitiveMark(row, channel);
  if (pinned) row.appendChild(pinMark("channel-pin"));
  paintChannelBand(row, channel);
  if (state.currentChannel && channel.id === state.currentChannel.id) row.classList.add("active");
  if (shownUnread(state.currentServer.id, channel.id)) row.classList.add("unread");
  row.onclick = () => {
    state.historyAnchor = null;
    selectChannel(channel);
  };
  row.oncontextmenu = e => {
    e.preventDefault();
    openChannelMenu(channel, e.clientX, e.clientY);
  };
  if (canCreate) row.appendChild(channelGear(channel));
  channelBellSlot(row, channel);
  return row;
}

function voiceChannelRow(channel, canCreate, pinned = false) {
  const row = el("div", "channel voice-channel");
  row.dataset.room = channel.id;
  const icon = el("span", "channel-icon");
  icon.appendChild(voiceKindIcon(channel.kind, 18));
  row.appendChild(icon);
  row.appendChild(channelNameNode(channel, "channel-label"));
  channelLockMark(row, channel);
  if (pinned) row.appendChild(pinMark("channel-pin"));
  paintChannelBand(row, channel);
  const full = voiceFull(channel);
  if (channel.id === state.voiceChannelId) {
    row.classList.add("active", "connected");
    row.title = "Click to disconnect";
  } else {
    row.title = full ? "This channel is full" : isStageChannel(channel) ? "Click to join the audience" : "Click to join voice";
  }
  if (channel.activity && activityPillLabel(channel.activity)) {
    const pill = el("span", "voice-activity-pill");
    pill.appendChild(svgIcon(activityKindInfo(channel.activity).icon, 12));
    pill.appendChild(el("span", null, activityPillLabel(channel.activity)));
    pill.title = `${activityName(channel.activity)} is running here`;
    row.appendChild(pill);
  }
  const count = voiceCountLabel(channel);
  if (count) {
    const badge = el("span", "voice-count" + (channel.user_limit ? " limited" : "") + (full ? " full" : ""), count);
    if (channel.user_limit) badge.title = full ? "Full" : `${channel.voice_users.length} of ${channel.user_limit} people`;
    row.appendChild(badge);
  }
  row.onclick = () => {
    if (voiceDragHoldsClick()) return;
    if (channel.can_view === false) {
      toast(`Only ${accessWho(state.currentServer, channel.view_roles)} can join this channel`);
      return;
    }
    if (full) {
      toast("That voice channel is full");
      return;
    }
    toggleVoice(channel);
  };
  row.oncontextmenu = e => {
    e.preventDefault();
    openChannelMenu(channel, e.clientX, e.clientY);
  };
  if (canCreate) row.appendChild(channelGear(channel));
  channelBellSlot(row, channel);
  return row;
}

function appendVoiceUsers(list, channel) {
  if (narrowSplit()) {
    list.appendChild(voiceRings(channel));
    return;
  }
  const myName = state.user ? (state.user.display_name || state.user.username) : null;
  if (!isStageChannel(channel)) {
    for (const entry of stageRoster(channel)) list.appendChild(voiceUserRow(channel, entry, myName));
    return;
  }
  const { speakers, audience } = stageSections(channel);
  if (speakers.length || audience.length) list.appendChild(stageGroupLabel("Speakers", speakers.length));
  for (const entry of speakers) list.appendChild(voiceUserRow(channel, entry, myName, "speaker"));
  if (audience.length) list.appendChild(stageGroupLabel("Audience", audience.length));
  for (const entry of audience) list.appendChild(voiceUserRow(channel, entry, myName, "audience"));
}

function voiceRingEntries(channel) {
  if (!isStageChannel(channel)) return stageRoster(channel);
  const { speakers, audience } = stageSections(channel);
  return [...speakers, ...audience];
}

function voiceRings(channel) {
  const rings = el("div", "voice-rings");
  rings.dataset.room = channel.id;
  rings.append(...voiceRingCells(channel));
  return rings;
}

function voiceRingCells(channel) {
  const entries = voiceRingEntries(channel);
  const { shown, more } = voiceRingSplit(entries.length);
  const cells = entries.slice(0, shown).map(entry => voiceRing(channel, entry));
  if (more) {
    const extra = el("span", "voice-ring voice-ring-more", `+${more}`);
    extra.title = entries.slice(shown).map(e => e.name).join(", ");
    cells.push(extra);
  }
  return cells;
}

function voiceRing(channel, entry) {
  const ring = el("span", "voice-user voice-ring");
  ring.dataset.room = channel.id;
  ring.title = entry.muted ? `${entry.name} (muted)` : entry.name;
  if (entry.id) {
    ring.dataset.user = entry.id;
    ring.classList.toggle("speaking", state.speaking.has(entry.id));
  }
  ring.classList.toggle("muted", Boolean(entry.muted));
  ring.appendChild(makeAvatar(entry.id, entry.name, "avatar avatar-sm"));
  const isMe = state.user && entry.id === state.user.id;
  ring.onclick = e => {
    e.stopPropagation();
    if (isMe) {
      openSelfVoiceMenu(e.clientX, e.clientY);
      return;
    }
    const server = state.currentServer;
    const member = server && server.members.find(m => m.id === entry.id);
    openMemberMenu(member || { id: entry.id, name: entry.name }, e.clientX, e.clientY);
  };
  return ring;
}

function stageGroupLabel(title, count) {
  const label = el("div", "stage-group");
  label.appendChild(el("span", "stage-group-title", title));
  label.appendChild(el("span", "stage-group-count", String(count)));
  return label;
}

function voiceUserRow(channel, entry, myName, role = null) {
  const { id: userId, name: user, since, muted, deafened } = entry;
  const isMe = state.user && userId ? userId === state.user.id : user === myName;
  const vu = el("div", "voice-user");
  vu.dataset.room = channel.id;
  if (userId) {
    vu.dataset.user = userId;
    vu.classList.toggle("speaking", state.speaking.has(userId));
    vu.classList.toggle("dragging", Boolean(voiceDrag && voiceDrag.live && voiceDrag.userId === userId));
  }
  vu.appendChild(makeAvatar(userId, user, "avatar avatar-sm"));
  vu.appendChild(el("span", "voice-user-name", user));
  decorateVoiceUser(vu, channel, userId);
  if (since) vu.appendChild(liveElapsed(since, "clock", "voice-user-time"));
  if (role) {
    const slot = el("span", "hand-slot");
    if (entry.hand) {
      slot.appendChild(svgIcon("hand", 14));
      slot.title = "Wants to speak";
      vu.classList.add("hand");
    }
    vu.appendChild(slot);
  }
  vu.appendChild(el("span", "vs-icon")).appendChild(svgIcon("mic", 14));
  if (role === "audience") {
    vu.classList.add("audience");
    setVoiceStatus(vu.lastChild, "micOff", "In the audience");
    vu.lastChild.classList.remove("off");
  }
  if (!isMe && muted && role !== "audience") setVoiceStatus(vu.lastChild, "micOff", "Muted");
  if (!isMe && deafened) {
    vu.appendChild(el("span", "vs-icon"));
    setVoiceStatus(vu.lastChild, "headsetOff", "Deafened");
  }
  const server = state.currentServer;
  if (userId && server && (server.server_muted || []).includes(userId)) {
    vu.appendChild(el("span", "vs-icon forced"));
    setVoiceStatus(vu.lastChild, "micOff", "Server Muted");
  }
  if (userId && server && (server.server_deafened || []).includes(userId)) {
    vu.appendChild(el("span", "vs-icon forced"));
    setVoiceStatus(vu.lastChild, "headsetOff", "Server Deafened");
  }
  if (userId && (state.voice.muted_users || []).includes(userId)) vu.classList.add("muted-for-me");
  if (userId && !isMe) {
    vu.onclick = vu.oncontextmenu = e => {
      e.preventDefault();
      e.stopPropagation();
      if (voiceDragHoldsClick()) return;
      const member = server && server.members.find(m => m.id === userId);
      openMemberMenu(member || { id: userId, name: user }, e.clientX, e.clientY);
    };
    if (canDragVoiceMember(server, userId, state.user && state.user.id)) {
      vu.onpointerdown = e => armVoiceDrag(e, vu, channel.id, userId, user);
    }
  } else if (isMe) {
    vu.onclick = vu.oncontextmenu = e => {
      e.preventDefault();
      e.stopPropagation();
      openSelfVoiceMenu(e.clientX, e.clientY);
    };
  }
  if (isMe && channel.id === state.voiceChannelId && role !== "audience") {
    if (state.voice.muted) setVoiceStatus(vu.lastChild, "micOff", "Muted");
    if (state.voice.deafened) {
      vu.appendChild(el("span", "vs-icon"));
      setVoiceStatus(vu.lastChild, "headsetOff", "Deafened");
    }
  }
  return vu;
}

let voiceDrag = null;
let voiceDragClickGuard = 0;

function voiceDragHoldsClick() {
  return Boolean(voiceDrag && voiceDrag.live) || Date.now() < voiceDragClickGuard;
}

function armVoiceDrag(e, row, fromId, userId, name) {
  if (e.button !== 0 || voiceDrag) return;
  voiceDrag = { pointerId: e.pointerId, kind: e.pointerType, x: e.clientX, y: e.clientY, at: Date.now(), row, fromId, userId, name, live: false, ghost: null, over: null, timer: null };
  if (e.pointerType === "touch") voiceDrag.timer = setTimeout(() => stepVoiceDrag(voiceDrag.x, voiceDrag.y), VOICE_DRAG.touchHoldMs);
  document.addEventListener("pointermove", onVoiceDragMove);
  document.addEventListener("pointerup", onVoiceDragUp);
  document.addEventListener("pointercancel", onVoiceDragCancel);
  document.addEventListener("touchmove", holdVoiceDragTouch, { passive: false });
  document.addEventListener("keydown", onVoiceDragKey);
}

function onVoiceDragMove(e) {
  if (!voiceDrag || e.pointerId !== voiceDrag.pointerId) return;
  stepVoiceDrag(e.clientX, e.clientY);
}

function stepVoiceDrag(x, y) {
  const drag = voiceDrag;
  if (!drag) return;
  if (!drag.live) {
    const step = voiceDragStep(drag.kind, x - drag.x, y - drag.y, Date.now() - drag.at);
    if (step === "cancel") return endVoiceDrag();
    if (step === "wait") return;
    startVoiceDrag(drag);
  }
  drag.ghost.style.transform = `translate(${Math.round(x + 12)}px, ${Math.round(y + 12)}px)`;
  const list = $("channel-list");
  const box = list.getBoundingClientRect();
  if (x >= box.left && x <= box.right) list.scrollTop += voiceEdgeScroll(y, box.top, box.bottom);
  markVoiceDrop(voiceDropAt(x, y));
}

function startVoiceDrag(drag) {
  clearTimeout(drag.timer);
  drag.live = true;
  closeFloating();
  const ghost = el("div", "voice-drag-ghost");
  ghost.appendChild(makeAvatar(drag.userId, drag.name, "avatar avatar-sm"));
  ghost.appendChild(el("span", "voice-user-name", drag.name));
  document.body.appendChild(ghost);
  drag.ghost = ghost;
  document.body.classList.add("voice-dragging");
  for (const node of document.querySelectorAll(`.voice-user[data-user="${CSS.escape(drag.userId)}"]`)) node.classList.add("dragging");
}

function voiceDropAt(x, y) {
  const hit = document.elementFromPoint(x, y);
  const node = hit && hit.closest("#channel-list [data-room]");
  return node ? voiceDropRoom(state.currentServer, voiceDrag.fromId, node.dataset.room) : null;
}

function markVoiceDrop(room) {
  const id = room ? room.id : null;
  if (voiceDrag.over === id) return;
  voiceDrag.over = id;
  for (const node of document.querySelectorAll(".voice-channel.voice-drop")) node.classList.remove("voice-drop");
  if (id) for (const node of document.querySelectorAll(`.voice-channel[data-room="${CSS.escape(id)}"]`)) node.classList.add("voice-drop");
}

function holdVoiceDragTouch(e) {
  if (voiceDrag && voiceDrag.live && e.cancelable) e.preventDefault();
}

function onVoiceDragUp(e) {
  if (!voiceDrag || e.pointerId !== voiceDrag.pointerId) return;
  const drag = voiceDrag;
  const room = drag.live ? voiceDropAt(e.clientX, e.clientY) : null;
  endVoiceDrag();
  if (!room) return;
  const server = state.currentServer;
  const member = (server && server.members.find(m => m.id === drag.userId)) || { id: drag.userId, name: drag.name };
  moveVoiceMember(member, room);
}

function onVoiceDragCancel(e) {
  if (voiceDrag && e.pointerId === voiceDrag.pointerId) endVoiceDrag();
}

function onVoiceDragKey(e) {
  if (e.key === "Escape" && voiceDrag) endVoiceDrag();
}

function endVoiceDrag() {
  const drag = voiceDrag;
  if (!drag) return;
  clearTimeout(drag.timer);
  voiceDrag = null;
  document.removeEventListener("pointermove", onVoiceDragMove);
  document.removeEventListener("pointerup", onVoiceDragUp);
  document.removeEventListener("pointercancel", onVoiceDragCancel);
  document.removeEventListener("touchmove", holdVoiceDragTouch);
  document.removeEventListener("keydown", onVoiceDragKey);
  if (!drag.live) return;
  voiceDragClickGuard = Date.now() + VOICE_DRAG.clickGuardMs;
  drag.ghost.remove();
  document.body.classList.remove("voice-dragging");
  for (const node of document.querySelectorAll(".voice-user.dragging")) node.classList.remove("dragging");
  for (const node of document.querySelectorAll(".voice-channel.voice-drop")) node.classList.remove("voice-drop");
}

const serverImages = new Map();

function loadServerImage(ref) {
  if (!ref) return Promise.resolve(null);
  return cachedLoad(serverImages, ref, () => invoke("fetch_server_image", { image: ref }));
}

function paintServerIcon(node, server) {
  node.replaceChildren(document.createTextNode(initials(server.name)));
  node.classList.remove("has-image");
  if (!server.icon) return;
  const ref = server.icon;
  loadServerImage(ref).then(url => {
    if (!url || server.icon !== ref) return;
    const img = el("img");
    img.src = url;
    img.alt = "";
    img.draggable = false;
    node.replaceChildren(img);
    node.classList.add("has-image");
  });
}

function paintSidebarBanner() {
  const header = $("server-name-header");
  if (plainServer()) {
    $("server-settings-btn").title = canOpenServerSettings(state.currentServer) ? "Server Settings" : "Server Info";
    $("invite-btn").classList.toggle("hidden", !state.currentServer.can_invite);
  }
  const banner = $("sidebar-banner");
  const server = plainServer();
  const ref = server ? server.banner : null;
  header.classList.toggle("has-banner", Boolean(server));
  header.classList.toggle("plain", Boolean(server) && !ref);
  banner.style.backgroundImage = "";
  header.style.setProperty("--slot", server ? starfield(server.id).css : "");
  if (!ref) return;
  loadServerImage(ref).then(url => {
    if (url && state.currentServer && state.currentServer.banner === ref) {
      banner.style.backgroundImage = `url("${url}")`;
    }
  });
}

function paintServerImages(server) {
  const canEdit = Boolean(server.can_manage_server);
  paintServerIcon($("overview-icon"), server);
  $("overview-icon").disabled = !canEdit;
  $("overview-icon-hint").classList.toggle("hidden", !canEdit);
  $("overview-icon-remove").classList.toggle("hidden", !(canEdit && server.icon));
  $("banner-upload").classList.toggle("hidden", !canEdit);
  $("banner-remove").classList.toggle("hidden", !(canEdit && server.banner));
  const preview = $("banner-preview");
  preview.style.backgroundImage = "";
  preview.classList.toggle("empty", !server.banner);
  if (server.banner) {
    const ref = server.banner;
    loadServerImage(ref).then(url => {
      if (url) preview.style.backgroundImage = `url("${url}")`;
    });
  }
}

function chooseImage(title, input) {
  if (IS_TAURI) return invoke("pick_image", { title });
  return new Promise(resolve => {
    input.value = "";
    input.onchange = () => resolve(input.files && input.files[0] ? URL.createObjectURL(input.files[0]) : null);
    input.click();
  });
}

function cropImageFile(file, width, height, quality = 0.88) {
  return new Promise((resolve, reject) => {
    const url = typeof file === "string" ? file : URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      const scale = Math.max(width / img.naturalWidth, height / img.naturalHeight);
      const sw = width / scale;
      const sh = height / scale;
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const ctx = canvas.getContext("2d");
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(img, (img.naturalWidth - sw) / 2, (img.naturalHeight - sh) / 2, sw, sh, 0, 0, width, height);
      URL.revokeObjectURL(url);
      resolve(canvas.toDataURL("image/jpeg", quality).split(",")[1]);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject("That file is not an image we can read");
    };
    img.src = url;
  });
}

async function setServerImage(which, data) {
  if (!state.currentServer) return;
  try {
    await invoke("set_server_image", { serverId: state.currentServer.id, which, data: data ?? null });
    toast(data ? ({ icon: "Server avatar updated", invite: "Invite background updated" }[which] || "Banner updated") : "Removed");
    await refreshServers();
    await refreshCurrentServer();
    openServerSettings();
  } catch (e) {
    toast(String(e));
  }
}

const SOUND_FILES = { notification: "sounds/notification.mp3", ding: "sounds/ding.mp3", pop: "sounds/pop.wav", join: "sounds/join.mp3", leave: "sounds/leave.mp3" };
const SOUND_EVENTS = [
  { id: "message", label: "New message or mention", fallback: "notification" },
  { id: "dm", label: "Direct message", fallback: "ding" },
  { id: "join", label: "Someone joins your voice channel", fallback: "join" },
  { id: "leave", label: "Someone leaves your voice channel", fallback: "leave" },
];
const SOUND_KEY = "commy.sounds";

function soundPrefs() {
  const defaults = { enabled: true, volume: 0.8, events: {} };
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(SOUND_KEY) || "{}") };
  } catch {
    return defaults;
  }
}

function saveSoundPrefs(prefs) {
  try { localStorage.setItem(SOUND_KEY, JSON.stringify(prefs)); } catch {}
}

function soundFor(eventId) {
  const event = SOUND_EVENTS.find(e => e.id === eventId);
  const chosen = soundPrefs().events[eventId];
  return chosen === undefined ? event?.fallback : chosen;
}

function playSound(eventId, force) {
  const prefs = soundPrefs();
  const name = force || soundFor(eventId);
  if (!name || name === "none" || !SOUND_FILES[name]) return;
  if (!force && (!prefs.enabled || state.user?.presence === "do_not_disturb")) return;
  const audio = new Audio(SOUND_FILES[name]);
  audio.volume = Math.max(0, Math.min(1, prefs.volume));
  audio.play().catch(() => {});
}

function renderSoundSettings() {
  const prefs = soundPrefs();
  $("sounds-toggle").checked = prefs.enabled;
  $("sound-volume").value = Math.round(prefs.volume * 100);
  $("sound-volume-label").textContent = `${$("sound-volume").value}%`;
  syncRangeFill($("sound-volume"));
  const rows = $("sound-rows");
  rows.replaceChildren();
  for (const event of SOUND_EVENTS) {
    const row = el("div", "sound-row");
    row.appendChild(el("span", "sound-label", event.label));
    const select = el("select", "compact-select");
    for (const name of [...Object.keys(SOUND_FILES), "none"]) {
      select.appendChild(new Option(name === "none" ? "No sound" : name[0].toUpperCase() + name.slice(1), name));
    }
    select.value = soundFor(event.id) || "none";
    select.onchange = () => {
      const next = soundPrefs();
      next.events[event.id] = select.value;
      saveSoundPrefs(next);
      playSound(event.id, select.value);
    };
    const preview = el("button", "icon-btn sound-preview");
    preview.title = "Preview";
    preview.appendChild(svgIcon("play", 18));
    preview.onclick = () => playSound(event.id, select.value);
    row.append(select, preview);
    rows.appendChild(row);
  }
  const board = soundboardPrefs();
  $("soundboard-toggle").checked = board.enabled;
  $("soundboard-volume").value = Math.round(board.volume * 100);
  $("soundboard-volume-label").textContent = `${$("soundboard-volume").value}%`;
  syncRangeFill($("soundboard-volume"));
}

let lastVoiceRoster = null;

function checkVoiceRoster() {
  const server = state.currentServer;
  const channel = server && state.voiceChannelId && server.channels.find(c => c.id === state.voiceChannelId);
  if (!channel) {
    lastVoiceRoster = null;
    return;
  }
  const others = new Set((channel.voice_user_ids || []).filter(id => !state.user || id !== state.user.id));
  const key = `${channel.id}`;
  if (lastVoiceRoster && lastVoiceRoster.key === key) {
    const joined = [...others].some(id => !lastVoiceRoster.ids.has(id));
    const left = [...lastVoiceRoster.ids].some(id => !others.has(id));
    if (joined) playSound("join");
    else if (left) playSound("leave");
  }
  lastVoiceRoster = { key, ids: others };
}

const NOTIFY_KEY = "commy.notify";

function notifyPrefs() {
  try {
    return { level: DEFAULT_NOTIFY_LEVEL, servers: {}, ...JSON.parse(localStorage.getItem(NOTIFY_KEY) || "{}") };
  } catch {
    return { level: DEFAULT_NOTIFY_LEVEL, servers: {} };
  }
}

function saveNotifyPrefs(prefs, { fromSync = false } = {}) {
  const before = fromSync ? null : notifyPrefs();
  try { localStorage.setItem(NOTIFY_KEY, JSON.stringify(prefs)); } catch {}
  if (fromSync) return;
  const changes = changedPieces(before, prefs);
  if (Object.keys(changes).length) invoke("sync_prefs_set", { changes }).catch(() => {});
}

function applySyncedPrefs(set) {
  saveNotifyPrefs(prefsWithPieces(notifyPrefs(), syncedValues(set)), { fromSync: true });
  $("notify-level").value = notifyPrefs().level;
  $("friend-online-toggle").checked = Boolean(notifyPrefs().friendOnline);
  renderServerRail();
  if (state.currentServer) renderChannels();
  if (!$("modal-server-notify").classList.contains("hidden")) renderServerNotify();
}

async function loadSyncedPrefs() {
  try {
    const { prefs, publish } = startupPrefs(notifyPrefs(), await invoke("sync_prefs_load"));
    saveNotifyPrefs(prefs, { fromSync: true });
    applySyncedPrefs({});
    if (Object.keys(publish).length) await invoke("sync_prefs_set", { changes: publish });
  } catch {}
}

let friendOnline = createFriendOnlineTracker();
const friendPresenceViews = new Map();

function noteFriendPresence(view) {
  if (!view || !view.id) return;
  friendPresenceViews.set(view.id, (view.members || []).map(m => ({ id: m.id, presence: m.presence })));
}

function showFriendOnline(friend) {
  const avatar = makeAvatar(friend.id, friend.name, "avatar toast-avatar");
  avatar.appendChild(el("span", "presence-dot online"));
  toast(`${friend.name} is now online`, { avatar, onClick: () => messageUser(friend.id, friend.name) });
  if (!appFocused()) invoke("notify", friendOnlineNotice(friend)).catch(() => {});
}

function announceFriendsOnline() {
  if (!state.user) return;
  const due = sweepFriendPresence(friendOnline, {
    friends: friends.friends,
    presenceOf: id => mergedPresence(friendPresenceViews.values(), id),
    prefs: notifyPrefs(),
    myPresence: state.user.presence,
    nowSecs: nowSecs(),
  });
  for (const friend of due) showFriendOnline(friend);
}

async function primeFriendPresence() {
  friendOnline = createFriendOnlineTracker();
  friendPresenceViews.clear();
  await refreshPeople();
  announceFriendsOnline();
}

async function trackFriendPresence(serverId) {
  if (!state.user || !friendOnline.swept || !friends.friends.length) return;
  if (state.currentServer && state.currentServer.id === serverId) noteFriendPresence(state.currentServer);
  else if (state.view !== "dm") {
    try {
      noteFriendPresence(await invoke("get_server", { serverId }));
    } catch {
      return;
    }
  }
  announceFriendsOnline();
}

function toggleFriendOnlinePick(userId) {
  const prefs = notifyPrefs();
  saveNotifyPrefs(withFriendOnlinePick(prefs, userId, !wantsFriendOnline(prefs, userId)));
}

const unread = { channels: new Map(), channelServer: new Map(), dms: new Map(), mentions: new Set() };
const READ_FLUSH_DELAY_MS = 2000;
const readState = { marks: {}, line: null, held: null, pending: {}, timer: null };

function myId() {
  return state.user ? state.user.id : null;
}

function skipsUnread(message) {
  return isBlocked(message.author_id);
}

function queueReadMark(channelId, mark, flush = false) {
  readState.pending[channelId] = mark;
  clearTimeout(readState.timer);
  readState.timer = null;
  if (flush) return flushReadMarks();
  readState.timer = setTimeout(flushReadMarks, READ_FLUSH_DELAY_MS);
}

function flushReadMarks() {
  clearTimeout(readState.timer);
  readState.timer = null;
  const channels = readState.pending;
  readState.pending = {};
  if (!Object.keys(channels).length) return;
  invoke("read_state_set", { channels, flush: true }).catch(() => {});
}

function advanceRead(channelId, messages, flush = false) {
  if (!channelId || readState.held === channelId) return;
  const mark = latestMark(messages);
  if (!markAdvances(readState.marks[channelId], mark)) return;
  readState.marks[channelId] = mark;
  queueReadMark(channelId, mark, flush);
}

function openChannelWithLine(channel) {
  if (readState.line && readState.line.channel === channel.id) return;
  if (readState.line && readState.line.channel !== channel.id) flushReadMarks();
  if (readState.held && readState.held !== channel.id) readState.held = null;
  readState.line = { channel: channel.id, mark: readState.marks[channel.id] || null };
}

function applyReadView(view) {
  const channels = (view && view.channels) || {};
  let changed = false;
  for (const [channelId, info] of Object.entries(channels)) {
    if (!info || !info.mark) continue;
    readState.marks[channelId] = newerMark(readState.marks[channelId], info.mark);
    const viewing = state.view === "server" && state.currentChannel && state.currentChannel.id === channelId;
    if (viewing && readState.held !== channelId) continue;
    if (info.unread > 0) {
      unread.channels.set(channelId, info.unread);
      if (info.server) unread.channelServer.set(channelId, info.server);
    } else {
      unread.channels.delete(channelId);
      unread.mentions.delete(channelId);
    }
    changed = true;
  }
  if (!changed) return;
  syncUnreadBadge();
  if (state.currentServer) renderChannels();
  if (dmListShown()) renderDmThreads();
}

async function loadReadState() {
  try {
    applyReadView(await invoke("read_state_load"));
  } catch {}
}

let listPins = { dms: [], channels: [] };

async function loadPins() {
  try {
    listPins = await invoke("pins_load");
  } catch {}
}

function dmIsPinned(userId) {
  return isPinned(listPins.dms, userId);
}

function channelIsPinned(serverId, channelId) {
  return isPinned(pinnedChannelIds(listPins, serverId), channelId);
}

async function setDmPinned(userId, pinned) {
  try {
    listPins = await invoke("pin_dm", { userId, pinned });
  } catch (e) {
    toast(String(e));
    return;
  }
  if (dmListShown()) renderDmThreads();
}

async function setChannelPinned(serverId, channelId, pinned) {
  try {
    listPins = await invoke("pin_channel", { serverId, channelId, pinned });
  } catch (e) {
    toast(String(e));
    return;
  }
  if (state.currentServer && state.currentServer.id === serverId) renderChannels();
}

function pinMark(cls) {
  const mark = el("span", cls);
  mark.title = "Pinned for you";
  mark.setAttribute("role", "img");
  mark.setAttribute("aria-label", "Pinned");
  mark.appendChild(svgIcon("pinOutline", 14));
  return mark;
}

function markUnreadFrom(message) {
  const messages = state.renderedMessages || [];
  const index = messages.findIndex(m => m.id === message.id);
  const channel = state.currentChannel;
  if (index < 0 || !channel || !state.currentServer) return;
  const mark = markBefore(messages, index);
  readState.marks[channel.id] = mark;
  readState.held = channel.id;
  readState.line = { channel: channel.id, mark };
  const count = unreadAfter(messages, mark, myId(), skipsUnread);
  if (count) {
    unread.channels.set(channel.id, count);
    unread.channelServer.set(channel.id, state.currentServer.id);
  } else {
    unread.channels.delete(channel.id);
  }
  queueReadMark(channel.id, mark, true);
  syncUnreadBadge();
  renderChannels();
  $("messages").dataset.view = "";
  renderMessages(messages);
}

async function markChannelsRead(serverId, channelIds) {
  for (const channelId of channelIds || [...unread.channelServer].filter(([, s]) => s === serverId).map(([c]) => c)) {
    if (readState.held === channelId) readState.held = null;
  }
  try {
    applyReadView(await invoke("read_state_mark_read", { serverId, channelIds: channelIds || null }));
  } catch {}
}

function appFocused() {
  return document.visibilityState === "visible" && document.hasFocus();
}

function phoneShowing() {
  return phoneInFront(document.body.classList.contains("mobile-app"), document.visibilityState);
}

function myMentionNames() {
  if (!state.user) return [];
  const me = state.view === "server" ? currentServerMember(state.user.id) : null;
  return [state.user.username, state.user.display_name, me && me.nickname];
}

function mentionsMe(content) {
  if (!state.user) return false;
  return mentionsName(content, myMentionNames());
}

function messageMentionsMeIn(serverId, authorId, content) {
  if (!state.user) return false;
  const server = state.currentServer && state.currentServer.id === serverId ? state.currentServer : null;
  return messageMentionsMe({ content, server, authorId, me: { id: state.user.id, names: myMentionNames() } });
}

function totalUnread() {
  let total = 0;
  for (const n of unread.channels.values()) total += n;
  for (const n of unread.dms.values()) total += n;
  return total;
}

function syncUnreadBadge() {
  invoke("set_unread", { count: totalUnread() }).catch(() => {});
  renderServerRail();
}

function clearChannelUnread(channelId) {
  unread.mentions.delete(channelId);
  if (unread.channels.delete(channelId)) syncUnreadBadge();
}

function clearDmUnread(userId) {
  if (unread.dms.delete(userId)) syncUnreadBadge();
}

const typingTracker = new TypingTracker();
const typingSender = new TypingSender(channel => {
  if (!state.currentServer) return;
  invoke("send_typing", { serverId: state.currentServer.id, channelId: channel }).catch(() => {});
});
let typingTimer = null;

function onTyping({ server, channel, user }) {
  if (!state.user || user === state.user.id || isBlocked(user)) return;
  if (!state.currentServer || server !== state.currentServer.id) return;
  typingTracker.start(channel, user);
  renderTypingStrip();
}

function renderTypingStrip() {
  const strip = $("typing-strip");
  const channel = state.view === "server" && state.currentChannel ? state.currentChannel.id : null;
  const typers = channel ? typingTracker.typers(channel) : [];
  const names = typers.map(id => {
    const member = currentServerMember(id);
    return member ? member.name : null;
  });
  const line = typingLine(names);
  const text = $("typing-text");
  text.replaceChildren();
  if (line) {
    const split = line.lastIndexOf(" are typing");
    const single = line.lastIndexOf(" is typing");
    const cut = split >= 0 ? split : single;
    if (cut > 0 && !line.startsWith("Several")) {
      text.appendChild(el("strong", null, line.slice(0, cut)));
      text.appendChild(document.createTextNode(line.slice(cut)));
    } else {
      text.textContent = line;
    }
  }
  strip.classList.toggle("on", Boolean(line));
  clearInterval(typingTimer);
  typingTimer = null;
  if (typingTracker.anyone()) typingTimer = setInterval(renderTypingStrip, 1000);
}

function noteTyped(input) {
  if (state.view !== "server" || !state.currentChannel) return;
  typingSender.typed(state.currentChannel.id, input.value);
}

function onMessageCreated(msg) {
  typingTracker.stop(msg.channel, msg.author);
  renderTypingStrip();
  if (state.user && msg.author === state.user.id) return;
  if (isBlocked(msg.author)) return;
  const viewing = state.view === "server" && state.currentChannel && state.currentChannel.id === msg.channel;
  const focusedOnIt = viewing && appFocused();
  const announcement = Boolean(msg.announcement);
  const prefs = notifyPrefs();
  const mentioned = typeof msg.mentioned === "boolean" ? mentionReaches(prefs, msg.server, msg) : messageMentionsMeIn(msg.server, msg.author, msg.content);
  if (!focusedOnIt) {
    unread.channels.set(msg.channel, (unread.channels.get(msg.channel) || 0) + 1);
    unread.channelServer.set(msg.channel, msg.server);
    if (mentioned) unread.mentions.add(msg.channel);
    if (viewing && readState.line && readState.line.channel === msg.channel && !readState.line.mark) {
      readState.line.mark = readState.marks[msg.channel] || null;
    }
    syncUnreadBadge();
    if (state.currentServer && state.currentServer.id === msg.server) renderChannels();
    else if (dmListShown() && isGroupServer(msg.server)) renderDmThreads();
  }
  const level = levelForChannel(prefs, msg.server, msg.channel, nowSecs());
  if (shouldNotify({ level, presence: state.user?.presence, focusedOnIt, phoneInFront: phoneShowing(), isDm: false, mentioned, announcement })) {
    const notice = messageNotice(sensitiveNotice(msg, msg.channel, sensitiveConfirmed()), plainText);
    invoke("notify", { ...notice, target: `channel:${msg.server}:${msg.channel}` }).catch(() => {});
    playSound("message");
  }
  speakIncoming(msg, viewing);
}

const TTS_KEY = "commy.tts";

function ttsPrefs() {
  try {
    return readTtsPrefs(localStorage.getItem(TTS_KEY));
  } catch {
    return readTtsPrefs(null);
  }
}

function saveTtsPrefs(prefs) {
  try { localStorage.setItem(TTS_KEY, JSON.stringify(prefs)); } catch {}
}

const speech = { native: false, ready: false, voices: [], loading: null, utterance: null };

function windowSpeech() {
  return typeof window !== "undefined" && Boolean(window.speechSynthesis) && typeof SpeechSynthesisUtterance === "function";
}

function speechAvailable() {
  return speech.native || windowSpeech();
}

function windowVoices() {
  if (!windowSpeech()) return [];
  return window.speechSynthesis.getVoices().map(v => ({ id: v.voiceURI || v.name, name: v.name, lang: v.lang, default: v.default }));
}

function loadSpeechVoices() {
  if (speech.loading) return speech.loading;
  speech.loading = invoke("speech_voices")
    .catch(() => null)
    .then(found => {
      speech.native = Boolean(found && found.native && Array.isArray(found.voices) && found.voices.length);
      if (!speech.native && windowSpeech()) {
        window.speechSynthesis.addEventListener("voiceschanged", () => {
          speech.voices = ttsVoices(windowVoices(), navigator.language);
          renderTtsSettings();
        });
      }
      speech.voices = ttsVoices(speech.native ? found.voices : windowVoices(), navigator.language);
      speech.ready = true;
      renderTtsSettings();
    });
  return speech.loading;
}

function stopSpeaking() {
  speech.utterance = null;
  if (speech.native) invoke("speech_stop").catch(() => {});
  if (windowSpeech()) window.speechSynthesis.cancel();
}

function speakInWindow(text, prefs) {
  const synth = window.speechSynthesis;
  const utterance = new SpeechSynthesisUtterance(text);
  const voice = chosenTtsVoice(speech.voices, prefs.voice);
  const match = voice && synth.getVoices().find(v => (v.voiceURI || v.name) === voice.id);
  if (match) {
    utterance.voice = match;
    utterance.lang = match.lang;
  }
  utterance.rate = prefs.rate;
  utterance.pitch = prefs.pitch;
  utterance.onend = utterance.onerror = () => { if (speech.utterance === utterance) speech.utterance = null; };
  speech.utterance = utterance;
  synth.cancel();
  setTimeout(() => {
    if (speech.utterance !== utterance) return;
    synth.resume();
    synth.speak(utterance);
  }, 60);
}

function speakNow(text) {
  const prefs = ttsPrefs();
  if (speech.native) {
    const voice = chosenTtsVoice(speech.voices, prefs.voice);
    invoke("speech_speak", { text, voice: voice ? voice.id : null, rate: prefs.rate, pitch: prefs.pitch }).catch(e => toast(`Could not read it aloud: ${e}`));
  } else if (windowSpeech()) {
    speakInWindow(text, prefs);
  }
}

function speakText(text) {
  if (!text) return false;
  if (!speech.ready) {
    loadSpeechVoices().then(() => (speechAvailable() ? speakNow(text) : toast("Text-to-speech is not available here")));
    return true;
  }
  if (!speechAvailable()) return false;
  speakNow(text);
  return true;
}

function speakMessage(message) {
  if (!speakText(spokenText(message.author_name, message.content, plainText))) toast("Text-to-speech is not available here");
}

function speakIncoming(msg, viewing) {
  const speak = shouldSpeakIncoming({
    mode: ttsPrefs().mode,
    tts: Boolean(msg.tts),
    ownMessage: Boolean(state.user && msg.author === state.user.id),
    deafened: Boolean(state.voice.deafened),
    viewing: Boolean(viewing),
    blocked: isBlocked(msg.author),
    presence: state.user?.presence,
    inVoice: Boolean(state.voiceChannelId),
  });
  if (speak) speakText(spokenText(msg.author_name, msg.content, plainText));
}

function canSendTtsHere() {
  return canSendTts(state.currentServer);
}

function renderTtsSettings() {
  const prefs = ttsPrefs();
  const usable = speechAvailable();
  $("tts-mode").value = prefs.mode;
  $("tts-rate").value = String(Math.round(prefs.rate * 10));
  $("tts-rate-label").textContent = `${prefs.rate.toFixed(1)}x`;
  $("tts-pitch").value = String(Math.round(prefs.pitch * 10));
  $("tts-pitch-label").textContent = `${prefs.pitch.toFixed(1)}`;
  const select = $("tts-voice");
  const options = [el("option", null, "Computer default")];
  options[0].value = "";
  for (const voice of speech.voices) {
    const option = el("option", null, ttsVoiceLabel(voice));
    option.value = voice.id;
    options.push(option);
  }
  select.replaceChildren(...options);
  select.value = chosenTtsVoice(speech.voices, prefs.voice) ? prefs.voice : "";
  for (const id of ["tts-mode", "tts-voice", "tts-rate", "tts-pitch", "tts-preview"]) $(id).disabled = !usable;
  $("tts-preview").title = usable ? "" : "Text-to-speech is not available here";
}

function onMessagesCaughtUp(payload) {
  const bareCountFromOlderBuild = count => ({ count: Number(count) });
  const server = payload && payload.server;
  const channels = payload && payload.channels;
  if (!server || !channels || typeof channels !== "object") return;
  let changed = false;
  const notices = [];
  const now = nowSecs();
  const prefs = notifyPrefs();
  for (const [channel, value] of Object.entries(channels)) {
    const info = typeof value === "object" && value ? value : bareCountFromOlderBuild(value);
    const count = Number(info.count);
    if (!Number.isInteger(count) || count <= 0) continue;
    const viewing = state.view === "server" && state.currentChannel && state.currentChannel.id === channel;
    const focusedOnIt = viewing && appFocused();
    const mentioned = mentionReaches(prefs, server, info) || mentionsMe(info.content);
    if (!focusedOnIt) {
      unread.channels.set(channel, (unread.channels.get(channel) || 0) + count);
      unread.channelServer.set(channel, server);
      if (mentioned) unread.mentions.add(channel);
      changed = true;
    }
    if (info.channel_name === undefined || isBlocked(info.author) || !isRecent(Number(info.at), now)) continue;
    const level = levelForChannel(prefs, server, channel, now);
    if (shouldNotify({ level, presence: state.user?.presence, focusedOnIt, phoneInFront: phoneShowing(), isDm: false, mentioned, announcement: Boolean(info.announcement) })) {
      const notice = caughtUpNotice(sensitiveNotice({ ...info, count, server_name: payload.server_name }, channel, sensitiveConfirmed()), plainText);
      notices.push({ ...notice, target: `channel:${server}:${channel}` });
    }
  }
  for (const notice of notices) invoke("notify", notice).catch(() => {});
  if (notices.length) playSound("message");
  if (!changed) return;
  syncUnreadBadge();
  if (state.currentServer && state.currentServer.id === server) renderChannels();
}

async function openNotificationTarget(target) {
  const [kind, a, b] = String(target || "").split(":");
  try {
    if (kind === "channel" && a && b) {
      if (!state.currentServer || state.currentServer.id !== a) await selectServer(a);
      const channel = state.currentServer && state.currentServer.channels.find(c => c.id === b);
      if (channel) await selectChannel(channel);
    } else if (kind === "dm" && a) {
      const thread = (await invoke("list_dm_threads")).find(t => t.user === a);
      await selectDm(a, thread ? thread.name : "Direct Message");
    }
    phoneShowChat();
  } catch (e) {
    toast(String(e));
  }
}

function onDmReceived(dm) {
  if (dm.request) {
    if (state.view === "dm") renderDmThreads();
    if (!dm.spam) toast(`Message request from ${dm.name}`);
    return;
  }
  const viewing = state.view === "dm" && state.currentDm && state.currentDm.user === dm.user;
  const focusedOnIt = viewing && appFocused();
  if (!focusedOnIt) {
    unread.dms.set(dm.user, (unread.dms.get(dm.user) || 0) + 1);
    syncUnreadBadge();
  }
  const prefs = notifyPrefs();
  if (shouldNotify({ level: prefs.level, presence: state.user?.presence, focusedOnIt, phoneInFront: phoneShowing(), isDm: true, mentioned: true })) {
    const body = plainText(dm.content) || (onlyAVoiceMessage(dm.content, dm.attachments) ? "Voice message" : "");
    invoke("notify", { title: dm.name, body, target: `dm:${dm.user}` }).catch(() => {});
    playSound("dm");
  }
}

const CAMERA_KEY = "commy.cameraDevice";
const CAMERA_FPS = 12;
const cam = { pipe: null, timer: null, busy: false, watching: new Set() };

function savedCameraId() {
  try { return localStorage.getItem(CAMERA_KEY) || ""; } catch { return ""; }
}

async function openCameraStream() {
  const deviceId = savedCameraId();
  const video = deviceId ? { deviceId: { exact: deviceId } } : {};
  try {
    return await navigator.mediaDevices.getUserMedia({
      video: { ...video, width: { ideal: 640 }, height: { ideal: 360 }, frameRate: { ideal: 15 } },
      audio: false,
    });
  } catch (e) {
    if (deviceId) return navigator.mediaDevices.getUserMedia({ video: true, audio: false });
    throw e;
  }
}

async function startCamera() {
  if (!state.voiceServer || cam.pipe) return;
  let stream;
  try {
    stream = await openCameraStream();
  } catch (e) {
    toast(`Camera unavailable: ${e.message || e}`);
    return;
  }
  try {
    await invoke("start_camera", { serverId: state.voiceServer.id });
  } catch (e) {
    for (const track of stream.getTracks()) track.stop();
    toast(String(e));
    return;
  }
  const pipe = createCameraPipeline(stream);
  cam.pipe = pipe;
  cam.timer = setInterval(async () => {
    if (cam.busy || !pipe.drawn || cam.pipe !== pipe) return;
    cam.busy = true;
    try {
      const { width, height } = pipe.canvas;
      const blob = await new Promise(r => pipe.canvas.toBlob(r, "image/jpeg", 0.7));
      if (blob && cam.pipe === pipe) {
        await invoke("push_camera_frame", { jpeg: await blobToBase64(blob), width, height });
      }
    } catch {}
    cam.busy = false;
  }, Math.round(1000 / CAMERA_FPS));
  state.cameraOn = true;
  renderVoicePanel();
  if (state.currentServer) await refreshCurrentServer();
}

function stopCameraTracks() {
  clearInterval(cam.timer);
  cam.timer = null;
  stopCameraPipeline(cam.pipe);
  cam.pipe = null;
}

async function stopCamera() {
  const server = state.voiceServer;
  stopCameraTracks();
  state.cameraOn = false;
  if (server) {
    try { await invoke("stop_camera", { serverId: server.id }); } catch {}
  }
  renderVoicePanel();
  if (state.currentServer) await refreshCurrentServer();
}

function cameraUsersHere() {
  const server = state.currentServer;
  if (!server || !state.voiceChannelId || !state.voiceServer || server.id !== state.voiceServer.id) return [];
  const channel = server.channels.find(c => c.id === state.voiceChannelId);
  if (!channel) return [];
  const on = new Set(server.cameras || []);
  const ids = channel.voice_user_ids || [];
  const users = ids
    .map((id, i) => ({ id, name: channel.voice_users[i] }))
    .filter(u => on.has(u.id));
  const me = state.user && users.findIndex(u => u.id === state.user.id);
  if (me > 0) users.unshift(...users.splice(me, 1));
  return users;
}

function renderCamGrid() {
  const grid = $("cam-grid");
  const users = cameraUsersHere();
  const wanted = new Set(users.map(u => u.id));
  for (const id of [...cam.watching]) {
    if (!wanted.has(id)) {
      cam.watching.delete(id);
      invoke("stop_watching_camera", { userId: id }).catch(() => {});
    }
  }
  grid.classList.toggle("hidden", !users.length);
  grid.dataset.count = String(Math.min(users.length, 4));
  for (const tile of [...grid.children]) {
    if (!wanted.has(tile.dataset.user)) tile.remove();
  }
  for (const user of users) {
    let tile = grid.querySelector(`[data-user="${CSS.escape(user.id)}"]`);
    const isMe = state.user && user.id === state.user.id;
    if (!tile) {
      tile = el("div", "cam-tile");
      tile.dataset.user = user.id;
      if (!isMe) {
        tile.appendChild(el("img"));
        tile.appendChild(el("div", "cam-wait", "Connecting"));
      }
      tile.appendChild(el("div", "cam-name", isMe ? `${user.name} (you)` : user.name));
      grid.appendChild(tile);
    }
    tile.classList.toggle("speaking", state.speaking.has(user.id));
    if (isMe) {
      if (cam.pipe && cam.pipe.canvas.parentNode !== tile) tile.prepend(cam.pipe.canvas);
    } else if (!cam.watching.has(user.id)) {
      cam.watching.add(user.id);
      invoke("watch_camera", { serverId: state.currentServer.id, userId: user.id }).catch(e => {
        cam.watching.delete(user.id);
        toast(String(e));
      });
    }
  }
}

function showCameraFrame(frame) {
  const tile = $("cam-grid").querySelector(`[data-user="${CSS.escape(frame.user)}"]`);
  if (!tile) return;
  const img = tile.querySelector("img");
  if (img) img.src = `data:image/jpeg;base64,${frame.jpeg}`;
  tile.querySelector(".cam-wait")?.remove();
}

async function loadCameraDevices() {
  const select = $("camera-device");
  try {
    const devices = (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === "videoinput");
    const chosen = savedCameraId();
    select.replaceChildren();
    select.appendChild(new Option("Default", ""));
    devices.forEach((d, i) => {
      select.appendChild(new Option(d.label || `Camera ${i + 1}`, d.deviceId));
    });
    select.value = devices.some(d => d.deviceId === chosen) ? chosen : "";
  } catch {}
}

let previewPipe = null;

async function toggleCameraPreview() {
  if (previewPipe) {
    stopCameraPreview();
    return;
  }
  try {
    previewPipe = createCameraPipeline(await openCameraStream());
    $("cam-stage").appendChild(previewPipe.canvas);
    $("cam-stage").classList.add("live");
    $("cam-preview-btn").textContent = "Stop Test";
    loadCameraDevices();
  } catch (e) {
    toast(`Camera unavailable: ${e.message || e}`);
  }
}

function stopCameraPreview() {
  stopCameraPipeline(previewPipe);
  previewPipe = null;
  $("cam-stage").classList.remove("live");
  $("cam-preview-btn").textContent = "Test Camera";
}

const CAMERA_FX_NOTES = {
  idle: "Effects run on this device. People in the call see what this preview shows.",
  "avatar-loading": "Getting your avatar ready. People in the call never see your camera picture.",
  "avatar-ready": "Your avatar follows your face on this device. People see it, never your camera.",
  "avatar-still": "Face tracking is not available here, so your avatar holds still.",
  "avatar-missing": "Choose a VRM avatar file to use your own. People see a plain background until then.",
  "avatar-failed": "Avatars are not available here, so your picture is blurred. Pick Off under Avatar.",
  loading: "Getting background effects ready. Your picture stays blurred until then.",
  ready: "Effects run on this device. People in the call see what this preview shows.",
  failed: "Effects are not available here, so your whole picture is blurred. Pick Off to send it clear.",
};

function renderCameraFx(status = cameraFxStatus()) {
  const { background, mirror } = cameraFx.settings;
  for (const button of $("cam-bg").querySelectorAll("button")) {
    const on = button.dataset.bg === background;
    button.classList.toggle("active", on);
    button.setAttribute("aria-checked", String(on));
  }
  renderAvatarPicker();
  $("cam-mirror").checked = mirror;
  document.body.dataset.camMirror = mirror ? "on" : "off";
  const note = $("cam-fx-note");
  note.textContent = CAMERA_FX_NOTES[status];
  note.classList.toggle("warn", status === "failed" || status === "avatar-failed");
}

function renderAvatarPicker() {
  const { avatar } = cameraFx.settings;
  for (const button of $("cam-avatar").querySelectorAll("button")) {
    const on = button.dataset.avatar === avatar;
    button.classList.toggle("active", on);
    button.setAttribute("aria-checked", String(on));
    const img = button.querySelector("img");
    const src = cameraAvatar.thumbnails[button.dataset.avatar];
    if (img && src && img.getAttribute("src") !== src) img.src = src;
    if (img && !src) img.removeAttribute("src");
  }
}

function pickAvatarBytes() {
  if (IS_TAURI) return invoke("pick_avatar_model");
  const input = $("vrm-file");
  return new Promise(resolve => {
    input.value = "";
    input.onchange = () => resolve(input.files && input.files[0] ? input.files[0].arrayBuffer() : null);
    input.click();
  });
}

async function chooseCameraAvatar() {
  try {
    const bytes = await pickAvatarBytes();
    if (!bytes || !bytes.byteLength) return;
    await setCustomAvatar(bytes);
    updateCameraFx({ avatar: "custom" });
  } catch (e) {
    toast(e && e.message ? e.message : String(e));
  }
}

async function pickCameraAvatar(kind) {
  if (kind === "custom") {
    await prepareAvatarStage();
    if (!cameraAvatar.stage || !cameraAvatar.stage.hasCustom()) {
      chooseCameraAvatar();
      return;
    }
  }
  updateCameraFx({ avatar: kind });
}

async function chooseCameraBackdrop() {
  try {
    const src = await chooseImage("Choose a camera background", $("server-image-file"));
    if (!src) return;
    setCameraBackdrop(await cropImageFile(src, CAMERA_BACKDROP_WIDTH, CAMERA_BACKDROP_HEIGHT));
    updateCameraFx({ background: "image" });
  } catch (e) {
    toast(String(e));
  }
}

function fillDeviceSelect(select, names, chosen) {
  select.replaceChildren();
  select.appendChild(new Option("Default", ""));
  for (const name of names) select.appendChild(new Option(name, name));
  if (chosen && !names.includes(chosen)) select.appendChild(new Option(`${chosen} (not connected)`, chosen));
  select.value = chosen || "";
}

async function loadAudioDevices() {
  try {
    const devices = await invoke("list_audio_devices");
    fillDeviceSelect($("input-device"), devices.inputs, state.voice.input_device);
    fillDeviceSelect($("output-device"), devices.outputs, state.voice.output_device);
  } catch (e) {
    toast(String(e));
  }
}

async function saveAudioDevices() {
  try {
    state.voice = await invoke("set_audio_devices", {
      input: $("input-device").value || null,
      output: $("output-device").value || null,
    });
    toast("Audio devices updated");
  } catch (e) {
    toast(String(e));
  }
}

function placePopup(popup, anchor, prefer = "above") {
  popup.classList.remove("hidden");
  if (anchor.closest && anchor.closest("#rail")) prefer = "below";
  const a = anchor.getBoundingClientRect();
  const p = popup.getBoundingClientRect();
  let top = prefer === "above" ? a.top - p.height - 8 : a.bottom + 8;
  if (top < 8) top = a.bottom + 8;
  if (top + p.height > window.innerHeight - 8) top = window.innerHeight - p.height - 8;
  let left = Math.min(prefer === "below" && anchor.closest && anchor.closest("#rail") ? a.right - p.width : a.left, window.innerWidth - p.width - 8);
  popup.style.top = `${Math.max(8, top)}px`;
  popup.style.left = `${Math.max(8, left)}px`;
}

function closeContextMenus() {
  for (const menu of document.querySelectorAll(".ctx-menu:not(.hidden)")) menu.classList.add("hidden");
}

function closeFloating(except) {
  if (except !== "profile-pop") closeDurations();
  if (except !== "conn-pop" && connTimer) closeConnPop();
  for (const id of ["profile-pop", "avatar-menu", "emoji-pop", "server-menu", "folder-menu", "channel-menu", "soundboard-pop", "member-menu", "msg-menu", "remind-pop", "member-card", "invite-menu", "audio-pop", "fp-menu", "composer-menu"]) {
    if (id !== except) $(id).classList.add("hidden");
  }
  if (except !== "pins-pop") closePinsPanel();
  if (except !== "threads-pop") closeThreadsPanel();
  if (except !== "search-pop") closeSearchPop();
}

const EMOJI_TONE_KEY = "commy.emoji.tone";
const EMOJI_FREQUENT_KEY = "commy.emoji.frequent";
const EMOJI_TONE_SAMPLE = "\u{270B}";
const EMOJI_PROBE_FONT = '100px "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif';
const EMOJI_SEQUENCE_WIDTH = 1.4;
const EMOJI_GROUP_ICONS = {
  "smileys-emotion": "emojiSmile", "people-body": "hand", "animals-nature": "emojiLeaf", "food-drink": "emojiFood",
  "travel-places": "emojiCar", activities: "emojiBall", objects: "emojiBulb", symbols: "emojiHeart", flags: "emojiFlag",
};
const emojiSupport = { version: null };
const emojiLookup = new Map();
const emojiBrowsers = {};

function bareEmoji(text) {
  return String(text || "").replace(/[\u{1F3FB}-\u{1F3FF}\uFE0F]/gu, "");
}

function emojiIndexOf(text) {
  if (!emojiLookup.size) EMOJI_DATA.forEach((row, i) => emojiLookup.set(bareEmoji(row[0]), i));
  return emojiLookup.get(bareEmoji(text));
}

function emojiProbe() {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  ctx.textBaseline = "top";
  ctx.font = EMOJI_PROBE_FONT;
  ctx.scale(0.01, 0.01);
  const pixel = (text, color) => {
    ctx.clearRect(0, 0, 100, 100);
    ctx.fillStyle = color;
    ctx.fillText(text, 0, 0);
    return ctx.getImageData(0, 0, 1, 1).data.join(",");
  };
  const single = ctx.measureText("\u{1F600}").width;
  return text => {
    const light = pixel(text, "#fff");
    const coloured = light === pixel(text, "#000") && !light.startsWith("0,0,0,");
    return coloured && ctx.measureText(text).width <= single * EMOJI_SEQUENCE_WIDTH;
  };
}

function detectEmojiSupport() {
  if (emojiSupport.version !== null) return emojiSupport;
  emojiSupport.version = Infinity;
  try {
    const renders = emojiProbe();
    emojiSupport.version = highestSupportedVersion(emojiVersionProbes(EMOJI_DATA), renders);
    if (renders("\u{1F600}") && !renders("\u{1F1FA}\u{1F1F8}")) document.documentElement.classList.add("emoji-flag-font");
  } catch {}
  return emojiSupport;
}

function usableEmojiRows() {
  const { version } = detectEmojiSupport();
  return version === Infinity ? EMOJI_DATA : EMOJI_DATA.filter(row => (row[5] || 0) <= version);
}

function emojiTone() {
  try {
    const tone = Number(localStorage.getItem(EMOJI_TONE_KEY));
    return Number.isInteger(tone) && tone >= 0 && tone <= EMOJI_TONES.length ? tone : 0;
  } catch {
    return 0;
  }
}

function setEmojiTone(tone) {
  try { localStorage.setItem(EMOJI_TONE_KEY, String(tone)); } catch {}
}

function frequentEmojiStore() {
  try { return readFrequentEmoji(localStorage.getItem(EMOJI_FREQUENT_KEY)); } catch { return {}; }
}

function rememberEmoji(value) {
  if (!value) return;
  try { localStorage.setItem(EMOJI_FREQUENT_KEY, JSON.stringify(recordEmojiUse(frequentEmojiStore(), value, Date.now()))); } catch {}
}

function tonedEmoji(entry, tone = emojiTone()) {
  return withSkinTone(entry.emoji, entry.skin, tone, EMOJI_TONES);
}

function buildEmojiBrowser(root, { placeholder = "Find the perfect emoji" } = {}) {
  const head = el("div", "eb-head");
  const field = el("label", "eb-search");
  const search = el("input");
  search.type = "text";
  search.placeholder = placeholder;
  search.autocomplete = "off";
  search.spellcheck = false;
  field.append(svgIcon("search", 16), search);
  const clear = el("button", "btn-secondary btn-small eb-clear hidden", "Remove");
  clear.type = "button";
  const tone = el("button", "eb-tone");
  tone.type = "button";
  tone.title = "Skin tone";
  const tones = el("div", "eb-tones hidden");
  tones.setAttribute("role", "listbox");
  tones.setAttribute("aria-label", "Skin tone");
  head.append(field, clear, tone, tones);
  const main = el("div", "eb-main");
  const rail = el("nav", "eb-rail");
  rail.setAttribute("aria-label", "Emoji categories");
  const body = el("div", "eb-body");
  main.append(rail, body);
  const foot = el("div", "eb-foot");
  const footGlyph = el("span", "eb-foot-glyph");
  const footName = el("span", "eb-foot-name");
  const footWhere = el("span", "eb-foot-where");
  foot.append(footGlyph, footName, footWhere);
  root.replaceChildren(head, main, foot);
  const b = { root, search, clear, tone, tones, rail, body, footGlyph, footName, footWhere, opts: {}, sections: [], frame: 0 };
  search.addEventListener("input", () => renderEmojiBrowser(b));
  search.addEventListener("keydown", e => {
    if (e.key !== "Enter") return;
    const first = b.body.querySelector("button[data-value], button[data-token]");
    if (first) {
      e.preventDefault();
      first.click();
    }
  });
  clear.onclick = () => {
    const onClear = b.opts.onClear;
    if (b.opts.onDone) b.opts.onDone();
    if (onClear) onClear();
  };
  tone.onclick = () => toggleEmojiTones(b);
  root.addEventListener("mousedown", e => {
    if (!b.tones.contains(e.target) && !tone.contains(e.target)) b.tones.classList.add("hidden");
  });
  body.addEventListener("click", e => {
    const btn = e.target.closest("button[data-value], button[data-token]");
    if (btn && body.contains(btn)) pickFromBrowser(b, btn.dataset.token || btn.dataset.value);
  });
  const hover = e => {
    const btn = e.target.closest("button[data-value], button[data-token]");
    if (btn && body.contains(btn)) showEmojiFoot(b, btn);
  };
  body.addEventListener("mouseover", hover);
  body.addEventListener("focusin", hover);
  body.addEventListener("scroll", () => {
    cancelAnimationFrame(b.frame);
    b.frame = requestAnimationFrame(() => markEmojiRail(b));
  });
  return b;
}

function openEmojiBrowser(b, opts) {
  b.opts = opts;
  b.search.value = "";
  b.clear.classList.toggle("hidden", !opts.onClear);
  b.tones.classList.add("hidden");
  paintToneButton(b);
  renderEmojiBrowser(b);
  b.body.scrollTop = 0;
  markEmojiRail(b);
}

function paintToneButton(b) {
  b.tone.textContent = withSkinTone(EMOJI_TONE_SAMPLE, SKIN_FIRST, emojiTone(), EMOJI_TONES);
}

function toggleEmojiTones(b) {
  const open = b.tones.classList.contains("hidden");
  b.tones.classList.toggle("hidden", !open);
  if (!open) return;
  const current = emojiTone();
  b.tones.replaceChildren();
  for (let tone = 0; tone <= EMOJI_TONES.length; tone++) {
    const option = el("button", "eb-tone-option" + (tone === current ? " on" : ""), withSkinTone(EMOJI_TONE_SAMPLE, SKIN_FIRST, tone, EMOJI_TONES));
    option.type = "button";
    option.setAttribute("role", "option");
    option.setAttribute("aria-selected", String(tone === current));
    option.onclick = () => {
      setEmojiTone(tone);
      b.tones.classList.add("hidden");
      paintToneButton(b);
      const top = b.body.scrollTop;
      renderEmojiBrowser(b);
      b.body.scrollTop = top;
    };
    b.tones.appendChild(option);
  }
}

function pickFromBrowser(b, value) {
  if (!value) return;
  rememberEmoji(value);
  const { onPick, onDone } = b.opts;
  if (onDone) onDone();
  if (onPick) onPick(value);
}

function customEmojiGroupsFor(custom) {
  if (!custom) return [];
  return expressionGroups("emojis").filter(g => g.local || custom !== "local");
}

function emojiSection(b, key, title, { icon = null, server = null, addKind = null, other = false, heading = true } = {}) {
  const section = el("section", "eb-section");
  section.dataset.key = key;
  if (heading) pickerSection(section, title, { other, addKind });
  const cells = el("div", "eb-cells");
  section.appendChild(cells);
  b.body.appendChild(section);
  b.sections.push({ key, title, icon, server, section });
  return cells;
}

function standardEmojiButton(value, index, label) {
  const btn = el("button", "eb-emoji", value);
  btn.type = "button";
  btn.dataset.value = value;
  btn.dataset.i = String(index);
  btn.setAttribute("aria-label", label);
  return btn;
}

function customEmojiButton(emoji, where) {
  const btn = el("button", "custom");
  btn.type = "button";
  btn.dataset.token = `<:${emoji.name}:${emoji.image}>`;
  btn.dataset.where = where || "";
  btn.setAttribute("aria-label", `:${emoji.name}:`);
  btn.appendChild(customEmojiImg(emoji.name, emoji.image));
  return btn;
}

function appendCustomGroupSection(b, group, items) {
  const server = state.servers.find(s => s.id === group.id) || { id: group.id, name: group.name };
  const addKind = b.opts.composer && group.local && canAddHere("emojis") && !b.search.value.trim() ? "emojis" : null;
  const cells = emojiSection(b, `server:${group.id}`, group.name, { server, addKind, other: !group.local });
  for (const emoji of items) cells.appendChild(customEmojiButton(emoji, group.name));
}

function renderEmojiBrowser(b) {
  const q = b.search.value.trim().toLowerCase();
  b.body.replaceChildren();
  b.sections = [];
  const groups = customEmojiGroupsFor(b.opts.custom);
  if (q) renderEmojiSearch(b, q, groups);
  else renderEmojiSections(b, groups);
  if (!b.body.childElementCount) b.body.appendChild(el("div", "picker-empty", "No matches"));
  if (!q) renderEmojiRail(b);
  markEmojiRail(b);
  showEmojiFoot(b, b.body.querySelector("button[data-value], button[data-token]"));
}

function appendFrequentSection(b, groups) {
  const tokens = new Map();
  for (const group of groups) for (const emoji of group.items) tokens.set(`<:${emoji.name}:${emoji.image}>`, { emoji, where: group.name });
  const version = detectEmojiSupport().version;
  const usable = key => tokens.has(key) || (!key.startsWith("<:") && emojiIndexOf(key) !== undefined && (EMOJI_DATA[emojiIndexOf(key)][5] || 0) <= version);
  const keys = frequentEmoji(frequentEmojiStore(), usable);
  if (!keys.length) return;
  const cells = emojiSection(b, "frequent", "Frequently Used", { icon: "emojiClock" });
  for (const key of keys) {
    const custom = tokens.get(key);
    if (custom) cells.appendChild(customEmojiButton(custom.emoji, custom.where));
    else cells.appendChild(standardEmojiButton(key, emojiIndexOf(key), EMOJI_DATA[emojiIndexOf(key)][1]));
  }
}

function appendComposerAddPrompt(b, groups) {
  if (!b.opts.composer || !canAddHere("emojis") || groups.some(g => g.local)) return;
  const here = serverHere();
  const cells = emojiSection(b, `server:${here.id}`, here.name, { server: here, addKind: "emojis" });
  cells.appendChild(el("div", "picker-empty", `No ${EXPRESSION_KINDS.emojis.many} here yet`));
}

function appendCatalogBrowserSection(b, q) {
  if (!b.opts.composer) return;
  const cells = emojiSection(b, "catalog", "emoji.gg", { icon: "globe", heading: false });
  if (!appendCatalogSection(cells, q, "emojis")) {
    b.sections.pop();
    cells.parentElement.remove();
  }
}

function renderEmojiSections(b, groups) {
  appendFrequentSection(b, groups);
  appendComposerAddPrompt(b, groups);
  for (const group of groups) appendCustomGroupSection(b, group, group.items);
  appendCatalogBrowserSection(b, "");
  const tone = emojiTone();
  if (b.standard && b.standard.tone === tone) {
    for (const cached of b.standard.sections) {
      b.body.appendChild(cached.section);
      b.sections.push(cached);
    }
    return;
  }
  const start = b.sections.length;
  for (const section of emojiByGroup(usableEmojiRows(), EMOJI_GROUPS)) {
    const cells = emojiSection(b, `group:${section.key}`, section.name, { icon: EMOJI_GROUP_ICONS[section.key] || "emojiSmile" });
    const frag = document.createDocumentFragment();
    for (const { entry } of section.items) frag.appendChild(standardEmojiButton(tonedEmoji(entry, tone), emojiIndexOf(entry.emoji), entry.label));
    cells.appendChild(frag);
  }
  b.standard = { tone, sections: b.sections.slice(start) };
}

function renderEmojiSearch(b, q, groups) {
  for (const group of groups) {
    const matches = expressionMatches(group.items, q.replace(/^:|:$/g, ""));
    if (matches.length) appendCustomGroupSection(b, group, matches);
  }
  appendCatalogBrowserSection(b, q);
  const found = searchEmoji(usableEmojiRows(), q);
  if (!found.length) return;
  const cells = emojiSection(b, "results", "Emoji", { icon: "emojiSmile" });
  const tone = emojiTone();
  for (const { entry } of found) cells.appendChild(standardEmojiButton(tonedEmoji(entry, tone), emojiIndexOf(entry.emoji), entry.label));
}

function emojiRailIcon(section) {
  if (!section.server) return svgIcon(section.icon || "emojiSmile", 20);
  const icon = el("span", "eb-rail-server");
  paintServerIcon(icon, section.server);
  return icon;
}

function renderEmojiRail(b) {
  b.rail.replaceChildren();
  let lastWasServer = null;
  for (const section of b.sections) {
    const isServer = Boolean(section.server) || section.key === "catalog";
    if (lastWasServer !== null && lastWasServer !== isServer) b.rail.appendChild(el("span", "eb-rail-sep"));
    lastWasServer = isServer;
    const btn = el("button", "eb-rail-btn");
    btn.type = "button";
    btn.dataset.key = section.key;
    btn.title = section.title || "";
    btn.setAttribute("aria-label", section.title || "");
    btn.appendChild(emojiRailIcon(section));
    btn.onclick = () => jumpToEmojiSection(b, section.key);
    b.rail.appendChild(btn);
  }
}

function jumpToEmojiSection(b, key) {
  if (b.search.value) {
    b.search.value = "";
    renderEmojiBrowser(b);
  }
  const section = b.sections.find(s => s.key === key);
  if (section) b.body.scrollTop = section.section.offsetTop;
  markEmojiRail(b, key);
}

function markEmojiRail(b, forced = null) {
  let key = forced;
  if (!key) {
    const top = b.body.scrollTop + 8;
    for (const section of b.sections) if (section.section.offsetTop <= top) key = section.key;
  }
  for (const btn of b.rail.querySelectorAll(".eb-rail-btn")) {
    const on = btn.dataset.key === key;
    btn.classList.toggle("active", on);
    if (on) keepRailButtonVisible(b.rail, btn);
  }
}

function keepRailButtonVisible(rail, btn) {
  const top = btn.offsetTop;
  const bottom = top + btn.offsetHeight;
  if (top < rail.scrollTop) rail.scrollTop = top;
  else if (bottom > rail.scrollTop + rail.clientHeight) rail.scrollTop = bottom - rail.clientHeight;
}

function showEmojiFoot(b, btn) {
  b.footGlyph.replaceChildren();
  b.footName.textContent = "";
  b.footWhere.textContent = "";
  if (!btn) return;
  if (btn.dataset.token) {
    const custom = parseEmojiToken(btn.dataset.token);
    if (!custom) return;
    b.footGlyph.appendChild(customEmojiImg(custom.name, custom.image, "custom-emoji eb-foot-img"));
    b.footName.textContent = `:${custom.name}:`;
    b.footWhere.textContent = btn.dataset.where ? `from ${btn.dataset.where}` : "";
    return;
  }
  const row = EMOJI_DATA[Number(btn.dataset.i)];
  b.footGlyph.textContent = btn.dataset.value;
  b.footName.textContent = row ? emojiCodesLabel(emojiRow(row)) : "";
}

function emojiPopBrowser() {
  if (!emojiBrowsers.pop) emojiBrowsers.pop = buildEmojiBrowser($("emoji-pop-browser"));
  return emojiBrowsers.pop;
}

function openEmojiPop(anchor, onPick, { custom = false, onClear = null, onGif = null } = {}) {
  const pop = $("emoji-pop");
  const b = emojiPopBrowser();
  const done = () => pop.classList.add("hidden");
  pop.classList.remove("hidden");
  popGifs.pick = onGif && ((gif, btn) => pickPopGif(gif, btn, onGif, done));
  popGifs.shown = false;
  $("emoji-pop-gif-search").value = "";
  $("emoji-pop-tabs").classList.toggle("hidden", !onGif);
  openEmojiBrowser(b, { custom, onPick, onClear, onDone: done });
  showEmojiPopTab("emoji");
  placePopup(pop, anchor, "above");
}

function showEmojiPopTab(tab) {
  const gifs = tab === "gifs";
  for (const btn of $("emoji-pop-tabs").querySelectorAll("button")) {
    btn.classList.toggle("active", btn.dataset.tab === tab);
    btn.setAttribute("aria-selected", String(btn.dataset.tab === tab));
  }
  $("emoji-pop-browser").classList.toggle("hidden", gifs);
  $("emoji-pop-gifs").classList.toggle("hidden", !gifs);
  const search = gifs ? $("emoji-pop-gif-search") : emojiPopBrowser().search;
  if (gifs && !popGifs.shown) {
    popGifs.shown = true;
    renderGifResults(search.value, popGifs);
  }
  if (!isPhoneWidth(window.innerWidth)) search.focus();
}

async function pickPopGif(gif, btn, onGif, done) {
  btn.classList.add("busy");
  try {
    await onGif(gif);
    done();
  } catch (e) {
    toast(String(e.message || e));
  } finally {
    btn.classList.remove("busy");
  }
}

function toggleProfilePopover() {
  const pop = $("profile-pop");
  if (!pop.classList.contains("hidden")) {
    pop.classList.add("hidden");
    return;
  }
  closeFloating("profile-pop");
  renderProfilePopover();
  placePopup(pop, document.querySelector(".user-panel"), "above");
}

function renderProfilePopover() {
  const user = state.user;
  if (!user) return;
  const name = user.display_name || user.username;
  $("pp-banner").dataset.banner = "";
  paintProfileBanner($("pp-banner"), user.banner, user.node, null, bannerColor(user.id));
  const av = $("pp-avatar");
  av.replaceChildren();
  paintAvatar(av, user.id, name);
  av.appendChild(el("span", "presence-dot " + (user.presence || "online")));
  $("pp-name").textContent = name;
  $("pp-username").textContent = user.username;
  $("pp-status").value = decodeCustomEmojis(user.custom_status);
  const emojiBtn = $("pp-emoji");
  if (user.status_emoji) setEmojiText(emojiBtn, user.status_emoji);
  else emojiBtn.replaceChildren(svgIcon("smile", 20));
  emojiBtn.classList.toggle("set", Boolean(user.status_emoji));
  $("pp-clear").classList.toggle("hidden", !(user.custom_status || user.status_emoji));
  for (const btn of $("pp-presence").querySelectorAll("button")) {
    btn.classList.toggle("active", btn.dataset.presence === (user.presence || "online"));
  }
  renderStatusExpiry();
  $("pp-switch-account").classList.toggle("hidden", !accountRows(account).length);
}

async function saveOwnProfile(changes) {
  const user = state.user;
  try {
    state.user = await invoke("update_profile", {
      displayName: user.display_name || null,
      customStatus: "customStatus" in changes ? changes.customStatus : user.custom_status || null,
      presence: changes.presence || user.presence || "online",
      statusEmoji: "statusEmoji" in changes ? changes.statusEmoji : undefined,
    });
    renderUserPanel();
    renderProfilePopover();
    if (state.currentServer) await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

let avatarMenuAnchor = null;
let accountStatusEmoji = "";

function openAvatarMenu(anchor) {
  avatarMenuAnchor = anchor;
  $("avatar-remove").classList.toggle("hidden", !(state.user && state.user.avatar));
  placePopup($("avatar-menu"), anchor, "below");
}

function imageFileToAvatar(file) {
  return new Promise((resolve, reject) => {
    const url = typeof file === "string" ? file : URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      const side = Math.min(img.naturalWidth, img.naturalHeight);
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 256;
      const ctx = canvas.getContext("2d");
      ctx.imageSmoothingQuality = "high";
      ctx.fillStyle = "#313338";
      ctx.fillRect(0, 0, 256, 256);
      ctx.drawImage(img, (img.naturalWidth - side) / 2, (img.naturalHeight - side) / 2, side, side, 0, 0, 256, 256);
      URL.revokeObjectURL(url);
      resolve(canvas.toDataURL("image/jpeg", 0.88).split(",")[1]);
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject("That file is not an image we can read");
    };
    img.src = url;
  });
}

async function pictureToAvatar(src, { keepStill = false } = {}) {
  const blob = await (await fetch(src)).blob();
  const plan = avatarPicturePlan(new Uint8Array(await blob.arrayBuffer()), { keepStill });
  if (plan === "keep") return blobToBase64(blob);
  if (plan === "too-large") throw new Error(`GIF avatars can be up to ${AVATAR_MAX_BYTES / 1024} KB. Try a smaller GIF.`);
  if (keepStill) return (await prepareEmojiImage(src, { size: 256, maxGif: 0 })).data;
  return imageFileToAvatar(src);
}

async function pickEmojiAvatar(picked) {
  const custom = parseEmojiToken(picked);
  if (!custom) return applyAvatar("emoji", picked);
  try {
    const src = await expressionUrl(custom.image);
    if (!src) throw new Error("That emoji could not be loaded right now. Try again in a moment.");
    await applyAvatar("image", await pictureToAvatar(src, { keepStill: true }));
  } catch (e) {
    toast(String(e.message || e));
  }
}

async function pickGifAvatar(gif) {
  const src = await invoke("fetch_catalog_image", { url: gif.preview || gif.url });
  await applyAvatar("image", await pictureToAvatar(src));
}

function avatarGifs() {
  return avatarGifsOffered(gifSettings(), serverHasGifs()) ? pickGifAvatar : null;
}

async function applyAvatar(kind, data) {
  try {
    state.user = await invoke("set_avatar", { kind, data: data ?? null });
    renderUserPanel();
    renderProfilePopover();
    if (!$("user-settings").classList.contains("hidden")) {
      const name = state.user.display_name || state.user.username;
      $("account-avatar").replaceChildren();
      paintAvatar($("account-avatar"), state.user.id, name);
    }
    if (state.currentServer) await refreshCurrentServer();
    toast(kind === "none" ? "Avatar removed" : "Avatar updated");
  } catch (e) {
    toast(String(e));
  }
}

let editingChannelId = null;

function channelGear(channel) {
  const gear = el("button", "channel-gear");
  gear.title = "Edit Channel";
  gear.appendChild(svgIcon("gear", 16));
  gear.onclick = e => {
    e.stopPropagation();
    openChannelSettings(channel.id);
  };
  return gear;
}

function editingChannel() {
  return state.currentServer && state.currentServer.channels.find(c => c.id === editingChannelId);
}

function openChannelSettings(channelId, tab = "overview") {
  editingChannelId = channelId;
  const channel = editingChannel();
  if (!channel) return;
  const voice = isVoiceKind(channel.kind);
  const announcement = channel.kind === "announcement";
  $("channel-settings-title").replaceChildren(...(voice || isForumChannel(channel) ? [] : [document.createTextNode("# ")]), channelNameNode(channel));
  $("channel-edit-prefix").replaceChildren(channelKindMark(channel.kind, 16));
  $("channel-announce-row").classList.toggle("hidden", voice || isForumChannel(channel));
  $("channel-edit-announce").checked = announcement;
  $("channel-sensitive-row").classList.toggle("hidden", !holdsMessages(channel));
  $("channel-edit-sensitive").checked = Boolean(channel.sensitive);
  const nameParts = channelLeadParts(channel.name);
  channelNameDrafts.edit = { lead: nameParts.lead, separator: nameParts.separator };
  $("channel-edit-name").value = nameParts.rest;
  paintEmojiSlot($("channel-edit-emoji"), nameParts.lead);
  const retention = String(channel.retention_days || 0);
  const select = $("channel-edit-retention");
  if (![...select.options].some(o => o.value === retention)) {
    select.appendChild(new Option(`${retention} days`, retention));
  }
  select.value = retention;
  $("channel-limit-row").classList.toggle("hidden", !voice);
  $("channel-limit").value = String(channel.user_limit || 0);
  syncLimitLabel();
  $("channel-slow-row").classList.toggle("hidden", !holdsMessages(channel));
  fillSlowModeSelect(channel.slow_mode || 0);
  channelColorDraft = channel.color || null;
  $("channel-edit-color").value = `#${channel.color || "9b7bff"}`;
  paintChannelBandPreview();
  syncChannelDirty();
  paintChannelBannerCard(channel);
  paintForumTagsCard(channel);
  $("channel-tab-permissions").classList.toggle("hidden", channel.kind === "category");
  accessDraft = { view: [...(channel.view_roles || [])], send: [...(channel.send_roles || [])] };
  paintChannelAccess();
  openSettingsLayer("channel-settings", tab);
}

let accessDraft = null;

function holdsAnyRole(server, ids) {
  const me = state.user && state.user.id;
  if (!me || server.owner === me) return true;
  const mine = (server.members.find(m => m.id === me) || {}).roles || [];
  const admin = server.roles.some(r => mine.includes(r.id) && (r.permissions || []).includes("administrator"));
  return admin || ids.some(id => mine.includes(id));
}

function paintAccessChips(container, server, ids, onChange) {
  container.replaceChildren();
  const everyone = el("button", "access-chip" + (ids.length ? "" : " picked"), "Everyone");
  everyone.type = "button";
  everyone.onclick = () => onChange([]);
  container.appendChild(everyone);
  for (const role of server.roles) {
    const chip = el("button", "access-chip" + (ids.includes(role.id) ? " picked" : ""));
    chip.type = "button";
    const dot = el("span", "role-dot");
    if (role.color) dot.style.background = `#${role.color}`;
    chip.append(dot, el("span", null, role.name));
    chip.onclick = () => onChange(toggleAccessRole(ids, role.id));
    container.appendChild(chip);
  }
}

function paintChannelAccess() {
  const channel = editingChannel();
  const server = state.currentServer;
  if (!channel || !server || !accessDraft) return;
  $("channel-view-title").textContent = isVoiceKind(channel.kind) ? "Who can see and join this channel" : "Who can see this channel";
  $("channel-send-card").classList.toggle("hidden", !holdsMessages(channel));
  paintAccessChips($("channel-view-roles"), server, accessDraft.view, ids => {
    accessDraft.view = ids;
    paintChannelAccess();
  });
  paintAccessChips($("channel-send-roles"), server, accessDraft.send, ids => {
    accessDraft.send = ids;
    paintChannelAccess();
  });
  let note = "";
  if (!server.roles.length) note = "Create roles in Server Settings to lock this channel to them.";
  else if (accessDraft.view.length && !holdsAnyRole(server, accessDraft.view)) note = "You don't hold any of these roles, so you won't be able to read this channel.";
  $("channel-access-note").textContent = note;
  const dirty = !sameAccess(accessDraft.view, channel.view_roles) || !sameAccess(accessDraft.send, channel.send_roles);
  $("channel-access-save-bar").classList.toggle("hidden", !dirty);
}

async function saveChannelAccess() {
  const channel = editingChannel();
  if (!channel || !accessDraft) return;
  try {
    await invoke("set_channel_access", {
      serverId: state.currentServer.id,
      channelId: channel.id,
      viewRoles: accessDraft.view,
      sendRoles: holdsMessages(channel) ? accessDraft.send : [],
    });
    toast(accessDraft.view.length ? `Only ${accessSummary(state.currentServer, accessDraft.view)} can see this channel now` : "Channel access saved");
    await refreshCurrentServer();
    openChannelSettings(channel.id, "permissions");
  } catch (e) {
    toast(String(e));
  }
}

let channelColorDraft = null;

function paintChannelBandPreview() {
  const preview = $("channel-band-preview");
  const channel = editingChannel();
  $("channel-band-sample").replaceChildren(channel ? channelNameNode(channel) : document.createTextNode("channel"));
  preview.classList.toggle("has-band", Boolean(channelColorDraft));
  preview.style.setProperty("--channel-band", channelColorDraft ? roleGradient(channelColorDraft) : "none");
  $("channel-edit-color-clear").classList.toggle("hidden", !channelColorDraft);
}

function paintChannelBannerCard(channel) {
  const preview = $("channel-banner-preview");
  preview.innerHTML = "";
  paintBanner(preview, channel.banner, "var(--bg-active)");
  if (!channel.banner) preview.appendChild(el("span", null, "No banner"));
  $("channel-banner-upload").textContent = channel.banner ? "Change Banner" : "Upload Banner";
  $("channel-banner-remove").classList.toggle("hidden", !channel.banner);
}

async function setChannelBanner(data) {
  const channel = editingChannel();
  if (!channel) return;
  try {
    await invoke("set_channel_banner", { serverId: state.currentServer.id, channelId: channel.id, data });
    await refreshCurrentServer();
    openChannelSettings(channel.id);
    if (state.currentChannel && state.currentChannel.id === channel.id) await selectChannel(editingChannel());
    toast(data ? "Channel banner saved" : "Channel banner removed");
  } catch (e) {
    toast(String(e));
  }
}

const channelNameDrafts = { edit: { lead: "", separator: null }, create: { lead: "", separator: null } };

function channelLeadParts(name) {
  return splitLeadingEmoji(name, emojiName => Boolean(serverEmoji(emojiName)));
}

function composeChannelName(draft, typed, textual) {
  let rest = typed.trim();
  if (textual) rest = rest.toLowerCase().replace(/\s+/g, "-");
  return joinLeadingEmoji(draft.lead, rest, draft.separator ?? (textual ? "-" : " "));
}

function paintEmojiSlot(btn, value) {
  btn.replaceChildren();
  btn.classList.toggle("set", Boolean(value));
  const token = parseEmojiToken(value);
  const named = !token && /^:([a-z0-9_]{2,32}):$/.exec(value || "");
  const custom = token || (named && serverEmoji(named[1]));
  if (custom) btn.appendChild(customEmojiImg(custom.name, custom.image));
  else if (value) btn.appendChild(el("span", "emoji-slot-glyph", value));
  else btn.appendChild(svgIcon("smilePlus", 20));
}

function pickChannelLead(anchor, which, changed) {
  const draft = channelNameDrafts[which];
  const set = lead => {
    draft.lead = lead;
    paintEmojiSlot(anchor, lead);
    changed();
  };
  openEmojiPop(anchor, picked => {
    const custom = parseEmojiToken(picked);
    set(custom ? `:${custom.name}:` : picked);
  }, { custom: "local", onClear: draft.lead ? () => set("") : null });
}

function insertEmojiInName(input, picked) {
  const custom = parseEmojiToken(picked);
  const text = custom ? `:${custom.name}:` : picked;
  const start = input.selectionStart ?? input.value.length;
  const end = input.selectionEnd ?? start;
  input.value = input.value.slice(0, start) + text + input.value.slice(end);
  const pos = start + text.length;
  input.focus();
  input.setSelectionRange(pos, pos);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function channelEdits() {
  const channel = editingChannel();
  if (!channel) return null;
  const name = composeChannelName(channelNameDrafts.edit, $("channel-edit-name").value, holdsMessages(channel));
  const retention = Number($("channel-edit-retention").value) || 0;
  const limit = isVoiceKind(channel.kind) ? Number($("channel-limit").value) || 0 : 0;
  const slow = holdsMessages(channel) ? Number($("channel-slow").value) || 0 : 0;
  const color = channelColorDraft;
  const dirty = name !== channel.name || retention !== (channel.retention_days || 0) || limit !== (channel.user_limit || 0)
    || slow !== (channel.slow_mode || 0) || color !== (channel.color || null);
  return { channel, name, retention, limit, slow, color, dirty };
}

function syncChannelDirty() {
  const edits = channelEdits();
  $("channel-save-bar").classList.toggle("hidden", !(edits && edits.dirty));
  $("channel-edit-save").disabled = !(edits && edits.name);
}

async function saveChannelEdits() {
  const edits = channelEdits();
  if (!edits || !edits.dirty || !edits.name) return;
  try {
    await invoke("update_channel", {
      serverId: state.currentServer.id,
      channelId: edits.channel.id,
      name: edits.name,
      retentionDays: edits.retention || null,
    });
    if (edits.limit !== (edits.channel.user_limit || 0)) {
      await invoke("set_channel_limit", { serverId: state.currentServer.id, channelId: edits.channel.id, limit: edits.limit });
    }
    if (edits.slow !== (edits.channel.slow_mode || 0)) {
      await invoke("set_channel_slow_mode", { serverId: state.currentServer.id, channelId: edits.channel.id, seconds: edits.slow });
    }
    if (edits.color !== (edits.channel.color || null)) {
      await invoke("set_channel_color", { serverId: state.currentServer.id, channelId: edits.channel.id, color: edits.color });
    }
    toast("Channel updated");
    await refreshCurrentServer();
    openChannelSettings(edits.channel.id);
  } catch (e) {
    toast(String(e));
  }
}

function fillSlowModeSelect(current) {
  const select = $("channel-slow");
  const values = SLOW_MODE_OPTIONS.includes(current) ? SLOW_MODE_OPTIONS : [...SLOW_MODE_OPTIONS, current].sort((a, b) => a - b);
  select.replaceChildren(...values.map(v => new Option(slowModeOptionText(v), String(v))));
  select.value = String(current);
}

const slowSent = new Map();
let slowTimer = null;

function noteSlowSend(channelId, waitSeconds) {
  const channel = state.currentChannel;
  const seconds = channel && channel.id === channelId ? Number(channel.slow_mode) || 0 : 0;
  if (!seconds) return;
  const at = waitSeconds ? Date.now() - (seconds - waitSeconds) * 1000 : Date.now();
  slowSent.set(channelId, at);
  syncSlowMode();
}

function syncSlowMode() {
  const pill = $("composer-slow");
  const channel = state.view === "server" ? state.currentChannel : null;
  const view = slowModeState({
    channel,
    canBypass: Boolean(state.currentServer && state.currentServer.can_bypass_slow_mode),
    sentAt: channel ? slowSent.get(channel.id) || 0 : 0,
    now: Date.now(),
  });
  pill.classList.toggle("hidden", !view.shown);
  pill.classList.toggle("waiting", view.waiting > 0);
  if (view.shown) {
    $("composer-slow-text").textContent = view.text;
    pill.title = view.waiting ? `Slow mode: you can send again in ${view.waiting}s` : `Slow mode: one message every ${slowModeOptionText(view.seconds)}`;
  }
  if (view.waiting > 0 && !slowTimer) slowTimer = setInterval(syncSlowMode, 1000);
  if (view.waiting === 0 && slowTimer) {
    clearInterval(slowTimer);
    slowTimer = null;
  }
}

function syncLimitLabel() {
  const n = Number($("channel-limit").value) || 0;
  $("channel-limit-label").textContent = n ? `${n} ${n === 1 ? "person" : "people"}` : "No limit";
}

function openDeleteChannel() {
  const channel = editingChannel();
  if (!channel) return;
  const label = isVoiceKind(channel.kind) ? channel.name : `#${channel.name}`;
  $("delete-channel-title").textContent = `Delete ${label}?`;
  $("delete-channel-name").textContent = channel.name;
  $("delete-channel-confirm").value = "";
  $("delete-channel-go").disabled = true;
  showModal("modal-delete-channel");
  $("delete-channel-confirm").focus();
}

async function confirmDeleteChannel() {
  const channel = editingChannel();
  if (!channel || $("delete-channel-confirm").value.trim() !== channel.name) return;
  try {
    await invoke("delete_channel", { serverId: state.currentServer.id, channelId: channel.id });
    hideModals();
    await closeSettingsLayers();
    toast(`Deleted ${isVoiceKind(channel.kind) ? channel.name : "#" + channel.name}`);
    editingChannelId = null;
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

let createChannelKind = "text";

let createChannelParent = null;

function openCreateChannel(kind, parent = null) {
  if (!state.currentServer || !state.currentServer.can_manage_channels) return;
  createChannelParent = parent;
  $("create-channel-name").value = "";
  channelNameDrafts.create = { lead: "", separator: null };
  paintEmojiSlot($("create-channel-emoji"), "");
  setCreateChannelKind(kind);
  showModal("modal-create-channel");
  $("create-channel-name").focus();
}

function setCreateChannelKind(kind) {
  createChannelKind = kind;
  for (const opt of $("create-channel-kinds").querySelectorAll(".kind-option")) {
    opt.classList.toggle("active", opt.dataset.kind === kind);
  }
  const stageHint = stageCreateHint(kind);
  $("create-channel-prefix").replaceChildren(channelKindMark(kind, 16));
  $("create-channel-name").placeholder = stageHint ? stageHint.placeholder : kind === "voice" ? "Hangout" : kind === "category" ? "Games" : kind === "announcement" ? "news" : kind === "forum" ? "ideas" : "new-channel";
  syncCreateChannelName();
}

function syncCreateChannelName() {
  const input = $("create-channel-name");
  if (kindHoldsMessages(createChannelKind)) {
    const pos = input.selectionStart;
    const cleaned = input.value.toLowerCase().replace(/\s+/g, "-");
    if (cleaned !== input.value) {
      input.value = cleaned;
      input.setSelectionRange(pos, pos);
    }
  }
  $("create-channel-save").disabled = !(input.value.trim() || channelNameDrafts.create.lead);
}

async function saveCreateChannel() {
  const name = composeChannelName(channelNameDrafts.create, $("create-channel-name").value, kindHoldsMessages(createChannelKind));
  if (!name || !state.currentServer) return;
  try {
    if (createChannelKind === "category") {
      await invoke("create_category", { serverId: state.currentServer.id, name });
    } else {
      await invoke("create_channel", { serverId: state.currentServer.id, name, kind: createChannelKind, parentId: createChannelParent });
    }
    hideModals();
    toast(`Created ${createChannelKind === "text" ? "#" : ""}${name}`);
    await refreshCurrentServer();
    if (kindHoldsMessages(createChannelKind)) {
      const made = state.currentServer.channels.filter(c => c.kind === createChannelKind).pop();
      if (made) await selectChannel(made);
    }
  } catch (e) {
    toast(String(e));
  }
}

function setSpeaking(userId, speaking) {
  if (speaking) state.speaking.add(userId);
  else state.speaking.delete(userId);
  for (const node of document.querySelectorAll(`[data-user="${CSS.escape(userId)}"]`)) {
    node.classList.toggle("speaking", speaking);
  }
  if (state.user && userId === state.user.id) {
    document.querySelector(".user-panel").classList.toggle("speaking", speaking);
  }
  paintAttenuation();
}

function clearSpeaking() {
  for (const userId of [...state.speaking]) setSpeaking(userId, false);
}

function renderMembers() {
  const list = $("member-list");
  list.innerHTML = "";
  if (!state.currentServer) return;
  rememberAvatars(state.currentServer.members);
  renderCamGrid();

  const sharers = new Set((state.currentServer.screen_shares || []).map(s => s.user));
  const { playing, rest } = splitPlaying(state.currentServer.members);
  renderPlayingGroup(list, playing);

  const group = (title, members, dim, color) => {
    if (!members.length) return;
    const heading = el("div", "member-group", `${title} - ${members.length}`);
    paintRole(heading, color);
    list.appendChild(heading);
    for (const member of members) {
      const row = el("div", "member" + (dim ? " offline" : ""));
      row.dataset.user = member.id;
      row.classList.toggle("speaking", state.speaking.has(member.id));
      const avatar = makeAvatar(member.id, member.name);
      const dot = el("span", "presence-dot " + member.presence);
      avatar.appendChild(dot);
      row.appendChild(avatar);
      const meta = el("div");
      const nameRow = el("div", "member-name");
      nameRow.appendChild(el("span", "member-name-text", member.name));
      paintRole(nameRow, member.color);
      if (member.bot) nameRow.appendChild(botTag());
      if (member.tag) nameRow.appendChild(roleTagChip(member.tag, member.tag_color));
      if (timeoutActive(member.timeout_until)) nameRow.appendChild(timeoutBadge(member.timeout_until));
      if (sharers.has(member.id)) {
        const live = el("span", "live-badge", "LIVE");
        live.title = "Watch screen";
        live.onclick = e => {
          e.stopPropagation();
          watchScreen(member.id, member.name);
        };
        nameRow.appendChild(live);
      }
      const channel = voiceChannelOf(member, state.currentServer);
      meta.appendChild(nameRow);
      const hover = memberHover(member, channel);
      if (hover) row.title = hover;
      const back = backLine(member);
      const text = back || statusLine(member);
      const line = el("div", "user-sub" + (member.activity && !back ? " activity" : "") + (text || channel ? "" : " empty"));
      if (channel) {
        const icon = el("span", "sub-voice");
        icon.appendChild(svgIcon("speaker", 14));
        line.appendChild(icon);
      }
      const cover = member.activity && !back && igdbCover(member.activity_cover, "t_thumb");
      if (cover) {
        const img = el("img", "activity-cover");
        img.src = cover;
        img.alt = "";
        line.appendChild(img);
      }
      if (text) line.appendChild(appendEmojiText(el("span", "sub-text"), text));
      meta.appendChild(line);
      row.appendChild(meta);
      row.onclick = e => {
        e.stopPropagation();
        openMemberCard(member, row);
      };
      if (!state.user || member.id !== state.user.id) {
        row.oncontextmenu = e => {
          e.preventDefault();
          openMemberMenu(member, e.clientX, e.clientY);
        };
      }
      list.appendChild(row);
    }
  };
  for (const g of groupMembers(rest, state.currentServer.roles)) group(g.title, g.members, !!g.dim, g.color);
}

function sensitiveConfirmed() {
  try {
    return readConfirmed(localStorage.getItem(SENSITIVE_KEY));
  } catch {
    return new Set();
  }
}

function ageGated(channel) {
  return sensitiveGated({ view: state.view, channel, confirmed: sensitiveConfirmed() });
}

function buildSensitiveGate(channel) {
  const wrap = el("div", "age-gate-wrap");
  const panel = el("div", "age-gate");
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-label", "Age-restricted channel");
  const icon = el("div", "age-gate-icon");
  icon.appendChild(svgIcon("warning", 40));
  const name = el("div", "age-gate-channel");
  name.append(document.createTextNode("#"), channelNameNode(channel));
  const text = sensitiveGateText(channelPlainName(channel));
  const actions = el("div", "age-gate-actions");
  const no = el("button", "btn-secondary age-gate-btn", "Nevermind");
  no.type = "button";
  no.onclick = leaveSensitiveChannel;
  const yes = el("button", "btn-primary age-gate-btn", "Continue");
  yes.type = "button";
  yes.onclick = () => confirmSensitiveChannel(channel);
  actions.append(no, yes);
  panel.append(icon, name, el("h2", null, text.title), el("p", null, text.body), actions);
  wrap.appendChild(panel);
  return wrap;
}

async function confirmSensitiveChannel(channel) {
  const confirmed = sensitiveConfirmed();
  confirmed.add(channel.id);
  try { localStorage.setItem(SENSITIVE_KEY, writeConfirmed(confirmed)); } catch {}
  $("messages").dataset.view = "";
  if (state.currentChannel && state.currentChannel.id === channel.id) await selectChannel(channel);
}

async function leaveSensitiveChannel() {
  if (!state.currentServer) return;
  const next = fallbackChannel({ channels: state.currentServer.channels, previous: state.previousChannel, confirmed: sensitiveConfirmed() });
  state.historyAnchor = null;
  await selectChannel(next);
}

async function resetSensitiveConfirmations() {
  try { localStorage.removeItem(SENSITIVE_KEY); } catch {}
  toast("Age-restricted channels will ask again");
  if (state.view === "server" && ageGated(state.currentChannel)) {
    $("messages").dataset.view = "";
    await selectChannel(state.currentChannel);
  }
}

function renderMessages(messages) {
  const box = $("messages");
  showForumView(false);
  if (state.view === "server" && ageGated(state.currentChannel)) {
    box.dataset.view = "";
    box.replaceChildren(buildSensitiveGate(state.currentChannel));
    state.renderedMessages = [];
    return;
  }
  const viewKey = state.view === "dm" ? `dm:${state.currentDm && state.currentDm.user}` : `ch:${state.currentChannel && state.currentChannel.id}`;
  const sameView = box.dataset.view === viewKey;
  const prevScroll = box.scrollTop;
  const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 60;
  box.dataset.view = viewKey;
  box.innerHTML = "";
  if (state.view === "dm" && state.currentDm) box.appendChild(buildDmIntro());
  else if (state.currentChannel && state.view === "server" && !canReadHistory(state.currentServer)) box.appendChild(historyNotice());
  else if (state.currentChannel) box.appendChild(buildChannelIntro(state.currentChannel));
  syncThreadsButton(messages);
  if (!messages.length) return;
  state.renderedMessages = messages;
  const replyCounts = state.view === "dm" ? new Map() : threadReplyCounts(messages);
  let lastAuthor = null;
  let lastTime = 0;
  let lastDay = null;
  let blockedRun = null;
  const lineAt = state.view === "server" && state.currentChannel
    ? newLineIndex(messages, lineFor(readState.line, state.currentChannel.id), myId(), skipsUnread)
    : -1;
  for (const [index, message] of messages.entries()) {
    const time = new Date(message.created_at).getTime();
    const day = new Date(message.created_at).toDateString();
    if (day !== lastDay) {
      box.appendChild(dayDivider(new Date(message.created_at)));
      lastDay = day;
      lastAuthor = null;
    }
    if (index === lineAt) box.appendChild(newDivider());
    if (state.view === "server" && isBlocked(message.author_id) && !revealedBlocked.has(message.id)) {
      if (!blockedRun) {
        blockedRun = [];
        box.appendChild(el("div", "blocked-slot"));
      }
      blockedRun.push(message);
      box.lastChild.replaceWith(blockedPlaceholder(blockedRun));
      lastAuthor = null;
      continue;
    }
    blockedRun = null;
    if (isHidden(message.id)) {
      box.appendChild(hiddenPlaceholder(message));
      lastAuthor = null;
      continue;
    }
    const grouped = !message.reply_to && message.author_id === lastAuthor && time - lastTime < 5 * 60e3;
    box.appendChild(buildMessageRow(message, grouped, { replies: replyCounts.get(message.id) || 0 }));
    lastAuthor = message.held ? null : message.author_id;
    lastTime = time;
  }
  if (state.view === "server" && state.historyAnchor && state.currentChannel && state.historyAnchor.channel === state.currentChannel.id) {
    box.appendChild(historyPill());
  }
  box.scrollTop = sameView && !atBottom ? prevScroll : box.scrollHeight;
  const line = !sameView && box.querySelector(".new-divider");
  if (line) {
    const above = line.getBoundingClientRect().top - box.getBoundingClientRect().top;
    if (above < 0) box.scrollTop += above - 8;
  }
  if (state.threadRoot) renderThread();
}

const FORUM_MODE_KEY = "commy.forumMode";
const forumUi = { mode: "list", filter: new Map(), draft: null, tagEmoji: null };

function storedForumMode() {
  try {
    return forumMode(localStorage.getItem(FORUM_MODE_KEY));
  } catch {
    return forumMode(null);
  }
}

function setForumMode(mode) {
  forumUi.mode = forumMode(mode);
  try { localStorage.setItem(FORUM_MODE_KEY, forumUi.mode); } catch {}
  for (const [id, value] of [["forum-mode-list", "list"], ["forum-mode-gallery", "gallery"]]) {
    $(id).classList.toggle("active", forumUi.mode === value);
    $(id).setAttribute("aria-pressed", String(forumUi.mode === value));
  }
  if (isForumChannel(state.currentChannel) && state.view === "server") renderForumPosts(state.currentChannel);
}

function showForumView(on) {
  $("forum-view").classList.toggle("hidden", !on);
  $("messages").classList.toggle("hidden", on);
}

function renderChannelLog(messages) {
  if (isForumChannel(state.currentChannel)) renderForum(messages);
  else renderMessages(messages);
}

function renderForum(messages) {
  const channel = state.currentChannel;
  state.renderedMessages = messages;
  showForumView(true);
  $("forum-new").classList.toggle("hidden", channel.can_send === false || channel.can_view === false);
  renderForumTags(channel);
  renderForumPosts(channel);
  if (state.threadRoot) renderThread();
}

function forumFilterFor(channel) {
  const picked = forumUi.filter.get(channel.id) || null;
  return picked && (channel.tags || []).some(t => t.id === picked) ? picked : null;
}

function renderForumTags(channel) {
  const row = $("forum-tags");
  row.replaceChildren();
  const picked = forumFilterFor(channel);
  const chip = (label, id) => {
    const button = el("button", "forum-chip" + (picked === id ? " picked" : ""), label);
    button.type = "button";
    button.setAttribute("role", "tab");
    button.setAttribute("aria-selected", String(picked === id));
    button.onclick = () => {
      if (id) forumUi.filter.set(channel.id, id);
      else forumUi.filter.delete(channel.id);
      renderForumTags(channel);
      renderForumPosts(channel);
    };
    return button;
  };
  row.appendChild(chip("All", null));
  for (const tag of channel.tags || []) row.appendChild(chip(tagLabel(tag), tag.id));
}

function forumTagPills(channel, ids) {
  const byId = forumTagsById(channel.tags);
  const wrap = el("span", "forum-pills");
  for (const id of ids || []) {
    const tag = byId.get(id);
    if (tag) wrap.appendChild(el("span", "forum-pill", tagLabel(tag)));
  }
  return wrap;
}

function forumPostMeta(post) {
  const meta = el("span", "forum-meta");
  meta.append(
    el("span", "forum-meta-author", post.author_name),
    el("span", "forum-meta-dot", "\u00b7"),
    el("span", null, replyCountLabel(post.replies)),
    el("span", "forum-meta-dot", "\u00b7"),
    el("span", null, activityLabel(post.last_activity)),
  );
  return meta;
}

function forumEditButton(post) {
  const me = state.user && state.user.id;
  const manages = Boolean(state.currentServer && state.currentServer.can_manage_messages);
  if (!canEditPost(post, me, manages)) return el("span", "forum-edit-slot");
  const button = el("button", "icon-btn forum-edit");
  button.type = "button";
  button.title = "Edit post";
  button.appendChild(svgIcon("pencil", 16));
  button.onclick = e => {
    e.stopPropagation();
    openForumComposer(post);
  };
  return button;
}

function forumOpenOn(node, post) {
  node.setAttribute("role", "button");
  node.tabIndex = 0;
  node.onclick = () => openForumPost(post);
  node.onkeydown = e => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      openForumPost(post);
    }
  };
}

function forumPostRow(channel, post) {
  const row = el("div", "forum-post" + (state.threadRoot === post.message ? " open" : ""));
  const main = el("div", "forum-post-main");
  const head = el("div", "forum-post-head");
  head.append(el("span", "forum-post-title", post.title), forumTagPills(channel, post.tags));
  main.append(head, forumPostMeta(post));
  row.append(main, forumEditButton(post));
  forumOpenOn(row, post);
  return row;
}

function forumPostBody(post) {
  const loaded = (state.renderedMessages || []).find(m => m.id === post.message);
  return loaded ? loaded.content : post.excerpt || "";
}

function forumPostCard(channel, post) {
  const card = el("div", "forum-card" + (state.threadRoot === post.message ? " open" : ""));
  const top = el("div", "forum-card-top");
  top.append(el("div", "forum-post-title", post.title), forumEditButton(post));
  const foot = el("div", "forum-card-foot");
  foot.append(forumTagPills(channel, post.tags), forumPostMeta(post));
  card.append(top, el("p", "forum-card-excerpt", postExcerpt(forumPostBody(post), 200) || "No text"), foot);
  forumOpenOn(card, post);
  return card;
}

function renderForumPosts(channel) {
  const box = $("forum-posts");
  box.className = `forum-posts ${forumUi.mode}`;
  box.replaceChildren();
  const all = channel.posts || [];
  const posts = sortPostsByActivity(filterPostsByTag(all, forumFilterFor(channel)));
  if (!posts.length) {
    const empty = el("div", "forum-empty");
    const mark = el("div", "channel-intro-mark");
    mark.appendChild(svgIcon("forum", 32));
    const filtered = all.length > 0;
    empty.append(
      mark,
      el("h2", null, filtered ? "No posts with this tag" : "No posts yet"),
      el("p", null, filtered ? "Pick another tag or show all posts." : channel.topic || "Start the first post. Replies gather under it like a thread."),
    );
    box.appendChild(empty);
    return;
  }
  for (const post of posts) box.appendChild(forumUi.mode === "gallery" ? forumPostCard(channel, post) : forumPostRow(channel, post));
}

async function openForumPost(post) {
  const channel = state.currentChannel;
  if (!channel) return;
  if (!(state.renderedMessages || []).some(m => m.id === post.message)) {
    state.historyAnchor = { channel: channel.id, message: post.message };
    const messages = await fetchChannelMessages(channel);
    if (state.currentChannel !== channel) return;
    renderForum(messages);
  }
  openThread(post.message);
  renderForumPosts(channel);
}

function forumPostFor(rootId) {
  const channel = state.currentChannel;
  if (!isForumChannel(channel)) return null;
  return (channel.posts || []).find(p => p.message === rootId) || null;
}

function openForumComposer(post = null) {
  const channel = state.currentChannel;
  if (!isForumChannel(channel)) return;
  forumUi.draft = { editing: post ? post.message : null, tags: post ? [...post.tags] : [] };
  $("forum-post-heading").textContent = post ? "Edit post" : "New post";
  $("forum-post-title").value = post ? post.title : "";
  $("forum-post-body").value = "";
  $("forum-post-body-label").classList.toggle("hidden", Boolean(post));
  $("forum-post-save").textContent = post ? "Save" : "Post";
  paintForumPostTags();
  syncForumDraft();
  showModal("modal-forum-post");
  $("forum-post-title").focus();
}

function paintForumPostTags() {
  const channel = state.currentChannel;
  const draft = forumUi.draft;
  const box = $("forum-post-tags");
  box.replaceChildren();
  if (!draft) return;
  const tags = (channel && channel.tags) || [];
  if (!tags.length) box.appendChild(el("span", "forum-no-tags", "This forum has no tags yet."));
  for (const tag of tags) {
    const chip = el("button", "access-chip" + (draft.tags.includes(tag.id) ? " picked" : ""), tagLabel(tag));
    chip.type = "button";
    chip.onclick = () => {
      draft.tags = togglePostTag(draft.tags, tag.id);
      paintForumPostTags();
      syncForumDraft();
    };
    box.appendChild(chip);
  }
  $("forum-post-tags-left").textContent = tags.length ? `${draft.tags.length} of ${MAX_POST_TAGS}` : "";
}

function forumDraftState() {
  const draft = forumUi.draft;
  if (!draft) return null;
  const body = draft.editing ? "kept" : $("forum-post-body").value;
  return postDraftState({ title: $("forum-post-title").value, body, tags: draft.tags });
}

function syncForumDraft() {
  const draft = forumDraftState();
  $("forum-post-save").disabled = !(draft && draft.canSave);
}

async function saveForumPost() {
  const draft = forumDraftState();
  const channel = state.currentChannel;
  if (!draft || !draft.canSave || !channel || !state.currentServer) return;
  const editing = forumUi.draft.editing;
  const args = { serverId: state.currentServer.id, channelId: channel.id, title: draft.title, tags: draft.tags };
  try {
    if (editing) await invoke("edit_forum_post", { ...args, messageId: editing });
    else await invoke("post_forum", { ...args, content: encodeOutgoing(draft.body) });
    hideModals();
    forumUi.draft = null;
    toast(editing ? "Post updated" : "Posted");
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

function setForumTagEmoji(picked) {
  forumUi.tagEmoji = picked || null;
  const button = $("channel-forum-emoji");
  button.replaceChildren(picked ? el("span", "forum-tag-emoji-pick", picked) : svgIcon("smile", 18));
  button.title = picked ? "Change the emoji" : "Pick an emoji";
}

function paintForumTagsCard(channel) {
  const forum = isForumChannel(channel);
  $("channel-forum-card").classList.toggle("hidden", !forum);
  if (!forum) return;
  const rows = $("channel-forum-tags");
  rows.replaceChildren();
  const tags = channel.tags || [];
  if (!tags.length) rows.appendChild(el("div", "forum-tag-empty", "No tags yet. Add a few so posts can be filtered."));
  tags.forEach((tag, i) => {
    const row = el("div", "forum-tag-row");
    const remove = el("button", "icon-btn forum-tag-remove");
    remove.type = "button";
    remove.title = "Remove tag";
    remove.appendChild(svgIcon("close", 16));
    remove.onclick = () => saveForumTags(tagsAfterRemove(tags, i));
    row.append(el("span", "forum-tag-row-emoji", tag.emoji || ""), el("span", "forum-tag-row-name", tag.name), remove);
    rows.appendChild(row);
  });
  $("channel-forum-name").value = "";
  $("channel-forum-add").disabled = true;
  setForumTagEmoji(null);
}

async function addForumTag() {
  const channel = editingChannel();
  if (!channel) return;
  const next = tagsAfterAdd(channel.tags || [], $("channel-forum-name").value, forumUi.tagEmoji);
  if (!next) {
    toast(`A tag with that name is already here, or the forum has its ${MAX_FORUM_TAGS} tags`);
    return;
  }
  await saveForumTags(next);
}

async function saveForumTags(tags) {
  const channel = editingChannel();
  if (!channel || !state.currentServer) return;
  try {
    await invoke("set_forum_tags", {
      serverId: state.currentServer.id,
      channelId: channel.id,
      tags: tags.map(t => ({ id: t.id || null, name: t.name, emoji: t.emoji || null })),
    });
    await refreshCurrentServer();
    openChannelSettings(channel.id);
  } catch (e) {
    toast(String(e));
  }
}

function wireForum() {
  setIcon($("forum-mode-list"), "list", 18);
  setIcon($("forum-mode-gallery"), "grid", 18);
  setForumMode(storedForumMode());
  $("forum-mode-list").onclick = () => setForumMode("list");
  $("forum-mode-gallery").onclick = () => setForumMode("gallery");
  $("forum-new").onclick = () => openForumComposer();
  $("forum-post-title").addEventListener("input", syncForumDraft);
  $("forum-post-body").addEventListener("input", syncForumDraft);
  $("forum-post-title").addEventListener("keydown", e => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (forumUi.draft && forumUi.draft.editing) saveForumPost();
      else $("forum-post-body").focus();
    }
  });
  $("forum-post-save").onclick = saveForumPost;
  $("forum-post-cancel").onclick = hideModals;
  $("channel-forum-name").addEventListener("input", () => {
    $("channel-forum-add").disabled = !cleanTagName($("channel-forum-name").value);
  });
  $("channel-forum-name").addEventListener("keydown", e => {
    if (e.key === "Enter" && !$("channel-forum-add").disabled) addForumTag();
  });
  $("channel-forum-add").onclick = addForumTag;
  $("channel-forum-emoji").onclick = e => openEmojiPop(e.currentTarget, picked => setForumTagEmoji(parseEmojiToken(picked) ? null : picked));
}

function buildGroupIntro(server) {
  const intro = el("div", "channel-intro");
  const mark = el("div", "channel-intro-mark");
  mark.appendChild(svgIcon("people", 34));
  intro.appendChild(mark);
  intro.appendChild(el("h2", null, server.name));
  const others = server.members.filter(m => !state.user || m.id !== state.user.id).map(m => m.name);
  const who = others.length > 1 ? `${others.slice(0, -1).join(", ")} and ${others[others.length - 1]}` : others[0] || "just you so far";
  intro.appendChild(el("p", null, `This is the start of your group with ${who}. Messages and calls go straight between the people in it.`));
  return intro;
}

function buildChannelIntro(channel) {
  if (isGroupView()) return buildGroupIntro(state.currentServer);
  const intro = el("div", "channel-intro");
  if (channel.banner) {
    const banner = el("div", "channel-intro-banner");
    paintBanner(banner, channel.banner, "var(--bg-raised)");
    intro.appendChild(banner);
  }
  const mark = el("div", "channel-intro-mark");
  const announcement = channel.kind === "announcement";
  if (isVoiceKind(channel.kind)) mark.appendChild(voiceKindIcon(channel.kind, 32));
  else if (announcement) mark.appendChild(svgIcon("megaphone", 34));
  else mark.appendChild(el("span", "channel-intro-hash", "#"));
  intro.appendChild(mark);
  const title = el("h2", null, isVoiceKind(channel.kind) ? "Welcome to " : "Welcome to #");
  title.appendChild(channelNameNode(channel));
  intro.appendChild(title);
  const fallback = announcement
    ? "News for everyone in the server. Only members allowed to post announcements write here."
    : "This is the start of the channel. Say something.";
  intro.appendChild(el("p", null, channel.topic || fallback));
  return intro;
}

function historyNotice() {
  const row = el("div", "history-notice");
  row.setAttribute("role", "status");
  row.appendChild(svgIcon("eyeOff", 16));
  row.appendChild(el("span", null, HISTORY_NOTICE));
  return row;
}

function dayDivider(date) {
  const row = el("div", "day-divider");
  row.appendChild(el("span", "day-divider-label", dayLabel(date)));
  return row;
}

function newDivider() {
  const row = el("div", "new-divider");
  row.setAttribute("role", "separator");
  row.setAttribute("aria-label", "New messages");
  row.appendChild(el("span", "new-divider-label", "NEW"));
  return row;
}

function isBareImageLink(content) {
  const text = (content || "").trim();
  if (findInviteLink(text)) return false;
  return /^https:\/\/\S+$/i.test(text) && imageLinks(text).length === 1;
}

const PENDING_TIP = "Waiting for someone to come online";

function pendingMark() {
  const mark = el("span", "msg-pending");
  mark.title = PENDING_TIP;
  mark.setAttribute("role", "img");
  mark.setAttribute("aria-label", PENDING_TIP);
  mark.appendChild(svgIcon("clock", 14));
  return mark;
}

function pinnedMark() {
  const mark = el("span", "msg-pinned");
  mark.title = "Pinned";
  mark.setAttribute("role", "img");
  mark.setAttribute("aria-label", "Pinned");
  mark.appendChild(svgIcon("pin", 13));
  mark.appendChild(document.createTextNode("Pinned"));
  return mark;
}

function ttsMark() {
  const mark = el("span", "msg-tts");
  mark.title = "Sent as text-to-speech. Read Aloud is in the message menu.";
  mark.setAttribute("role", "img");
  mark.setAttribute("aria-label", "Text-to-speech");
  mark.appendChild(svgIcon("speaker", 13));
  mark.appendChild(document.createTextNode("Spoken"));
  return mark;
}

function heldMark(reason) {
  const mark = el("span", "msg-held");
  mark.title = "Members don't see this message. Auto moderation held it.";
  mark.setAttribute("role", "img");
  mark.setAttribute("aria-label", heldLabel(reason));
  mark.appendChild(svgIcon("eyeOff", 13));
  mark.appendChild(document.createTextNode(heldLabel(reason)));
  return mark;
}

function buildMessageRow(message, grouped, { replies = 0, inThread = false } = {}) {
  const pending = Boolean(message.pending);
  const held = Boolean(message.held);
  if (held || message.tts) grouped = false;
  const row = el("div", "msg" + (grouped ? "" : " first") + (message.reply_to ? " is-reply" : "") + (pending ? " pending" : "") + (held ? " held" : "") + (message.tts ? " tts" : ""));
  row.dataset.id = message.id;
  row.appendChild(messageActions(message, inThread, replies));
  if (message.reply_to) row.appendChild(replyReference(message));

  const avatar = grouped ? el("div", "avatar spacer", "") : makeAvatar(message.author_id, message.author_name);
  row.appendChild(avatar);

  const body = el("div", "msg-body");
  if (!grouped) {
    const head = el("div", "msg-head");
    const author = el("span", "msg-author", message.author_name);
    head.appendChild(author);
    decorateAuthor(author, message.author_id);
    head.appendChild(el("span", "msg-time", formatTime(message.created_at)));
    if (message.pinned) head.appendChild(pinnedMark());
    if (message.tts) head.appendChild(ttsMark());
    if (held) head.appendChild(heldMark(message.held));
    if (pending) head.appendChild(pendingMark());
    body.appendChild(head);
  }
  if (message.pinned) row.classList.add("pinned");
  let inlineMark = pending && grouped;
  if (message.command) {
    const line = renderCommandUse(message);
    if (inlineMark) line.appendChild(pendingMark());
    inlineMark = false;
    body.appendChild(line);
  } else if (message.content) {
    if (!isBareImageLink(message.content)) {
      const content = buildMessageContent(message.content, message.author_id);
      if (message.edited) appendAfterText(content, el("span", "edited", " (edited)"));
      if (inlineMark) appendAfterText(content, pendingMark());
      inlineMark = false;
      body.appendChild(content);
    }
    if (showsEmbeds(message)) body.append(...imageEmbeds(imageLinks(message.content)));
  }
  if (linkPreviewPrefs().show && showsEmbeds(message)) {
    for (const preview of message.previews || []) body.appendChild(renderLinkPreview(preview));
  }
  if (message.poll) body.appendChild(renderPoll(message));
  if (message.voice_invite) body.appendChild(renderVoiceInvite(message));
  if (message.sealed) {
    const sealed = el("div", "msg-content msg-sealed");
    sealed.appendChild(svgIcon("lock", 14));
    sealed.appendChild(el("span", null, "Sealed message. It opens once this channel's key reaches you."));
    body.appendChild(sealed);
  }
  if (inlineMark) body.prepend(pendingMark());
  for (const att of (message.attachments || []).map(attachmentHere)) {
    body.appendChild(renderAttachment(att));
    if (att.is_image) body.appendChild(renderInlineImage(att));
    else if (playsInline(att)) body.appendChild(renderInlineVideo(att));
  }
  const pills = reactionPills(message);
  if (pills) body.appendChild(pills);
  if (replies && !inThread) {
    const link = el("button", "thread-link", `${replies} ${replies === 1 ? "reply" : "replies"}`);
    link.prepend(svgIcon("thread", 14));
    link.onclick = () => openThread(message.id);
    body.appendChild(link);
  }
  row.appendChild(body);
  return row;
}

async function setPinned(message, pinned) {
  if (!state.currentServer || !state.currentChannel) return;
  const channel = state.currentChannel;
  if (pinned && !pinRoomLeft(channel.pins)) {
    toast(`A channel holds up to ${MAX_PINS_PER_CHANNEL} pinned messages. Unpin one first.`);
    return;
  }
  try {
    await invoke("pin_message", { serverId: state.currentServer.id, channelId: channel.id, messageId: message.id, pinned });
    toast(pinned ? "Pinned" : "Unpinned");
    await refreshCurrentServer();
    if (!$("pins-pop").classList.contains("hidden")) await renderPinsPanel();
  } catch (e) {
    toast(String(e));
  }
}

const PINS_SEEN_KEY = "commy.pins.seen";
let pinSyncToken = 0;

function pinsSeen(channelId) {
  try {
    const all = JSON.parse(localStorage.getItem(PINS_SEEN_KEY) || "{}");
    return Array.isArray(all[channelId]) ? all[channelId] : [];
  } catch {
    return [];
  }
}

function markPinsSeen(channelId, pins) {
  try {
    const all = JSON.parse(localStorage.getItem(PINS_SEEN_KEY) || "{}");
    all[channelId] = (pins || []).map(p => p.message.id);
    localStorage.setItem(PINS_SEEN_KEY, JSON.stringify(all));
  } catch {}
}

function syncPinButton() {
  const btn = $("pins-btn");
  const channel = state.view === "server" ? state.currentChannel : null;
  const { visible, count, title } = pinButtonState({ view: state.view, channel, count: channel && channel.pins });
  btn.classList.toggle("hidden", !visible);
  btn.title = title;
  btn.classList.toggle("has-pins", count > 0);
  const badge = $("pins-count");
  badge.classList.add("hidden");
  const token = ++pinSyncToken;
  if (!visible) return closePinsPanel();
  if (count === 0) return;
  invoke("get_pins", { serverId: state.currentServer.id, channelId: channel.id }).then(pins => {
    if (token !== pinSyncToken) return;
    const unseen = unseenPinIds(pins, pinsSeen(channel.id)).length;
    badge.textContent = pinBadgeText(unseen);
    badge.classList.toggle("hidden", unseen === 0);
    btn.title = pinTitleWithNew(title, unseen);
  }).catch(() => {});
}

function closePinsPanel() {
  $("pins-pop").classList.add("hidden");
  $("pins-btn").classList.remove("active");
}

async function togglePinsPanel() {
  const pop = $("pins-pop");
  if (!pop.classList.contains("hidden")) return closePinsPanel();
  closeFloating();
  pop.classList.remove("hidden");
  $("pins-btn").classList.add("active");
  await renderPinsPanel();
}

async function renderPinsPanel() {
  const list = $("pins-list");
  if (!state.currentServer || !state.currentChannel) return;
  let pins = [];
  try {
    pins = await invoke("get_pins", { serverId: state.currentServer.id, channelId: state.currentChannel.id });
  } catch (e) {
    toast(String(e));
  }
  list.innerHTML = "";
  markPinsSeen(state.currentChannel.id, pins);
  $("pins-count").classList.add("hidden");
  $("pins-btn").title = pinButtonState({ view: state.view, channel: state.currentChannel, count: pins.length }).title;
  if (!pins.length) {
    list.appendChild(pinsEmptyState());
    return;
  }
  for (const pin of sortPins(pins)) list.appendChild(pinRow(pin, { jump: () => jumpToPin(pin, true) }));
}

function pinsEmptyState() {
  const empty = el("div", "pins-empty");
  const mark = el("div", "pins-empty-mark");
  mark.appendChild(svgIcon("pin", 26));
  empty.appendChild(mark);
  empty.appendChild(el("strong", null, "No pinned messages yet"));
  const hint = canPinIn(state.currentServer, state.currentChannel)
    ? "Hover a message and choose Pin to keep it here for everyone."
    : "Members who can pin messages keep the important ones here.";
  empty.appendChild(el("span", null, hint));
  return empty;
}

function pinRow(pin, { jump, showChannel = false } = {}) {
  const message = pin.message;
  const row = el("div", "pin-row");
  row.dataset.id = message.id;
  row.setAttribute("role", "button");
  row.tabIndex = 0;
  row.appendChild(makeAvatar(message.author_id, message.author_name, "avatar avatar-sm"));
  const body = el("div", "pin-body");
  const head = el("div", "pin-head");
  const author = el("span", "pin-author", message.author_name);
  decorateAuthor(author, message.author_id);
  head.appendChild(author);
  head.appendChild(el("span", "pin-time", formatTime(message.created_at)));
  if (showChannel) head.appendChild(el("span", "pin-channel", `#${pin.channel_name}`));
  body.appendChild(head);
  const pinChannel = state.currentServer && state.currentServer.channels.find(c => c.id === pin.channel);
  const text = sensitivePreview({ channel: pinChannel, confirmed: sensitiveConfirmed(), text: pinExcerpt(message, plainText) });
  const excerpt = el("div", "pin-text", text);
  if (!text || text === SENSITIVE_PREVIEW) excerpt.classList.add("muted");
  body.appendChild(excerpt);
  const images = (message.attachments || []).filter(a => a.is_image && !imagePreviewFailures.has(a.blob) && (loadsImageOnItsOwn(a) || imagePreviewCache.has(a.blob)));
  if (images.length) {
    const thumb = renderInlineImage(attachmentHere(images[0]));
    thumb.classList.add("pin-thumb");
    body.appendChild(thumb);
  }
  body.appendChild(el("div", "pin-by", `Pinned by ${pin.by_name}`));
  row.appendChild(body);
  if (state.currentServer && state.currentServer.can_pin) {
    const unpin = el("button", "pin-unpin");
    unpin.title = "Unpin";
    unpin.setAttribute("aria-label", "Unpin");
    unpin.appendChild(svgIcon("close", 16));
    unpin.onclick = async e => {
      e.stopPropagation();
      try {
        await invoke("pin_message", { serverId: state.currentServer.id, channelId: pin.channel, messageId: message.id, pinned: false });
        row.remove();
        await refreshCurrentServer();
        if (!$("pins-pop").classList.contains("hidden") && !$("pins-list").children.length) $("pins-list").appendChild(pinsEmptyState());
        if (!$("modal-pins").classList.contains("hidden")) await renderServerPins();
      } catch (err) {
        toast(String(err));
      }
    };
    row.appendChild(unpin);
  }
  row.onclick = jump;
  row.onkeydown = e => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      jump();
    }
  };
  return row;
}

async function jumpToPin(pin, fromPanel = false) {
  if (!state.currentServer) return;
  if (fromPanel) closePinsPanel();
  else hideModals();
  if (!state.currentChannel || state.currentChannel.id !== pin.channel) {
    const channel = state.currentServer.channels.find(c => c.id === pin.channel);
    if (!channel) {
      toast("That channel is gone");
      return;
    }
    await selectChannel(channel);
  }
  jumpToMessage(pin.message.id);
}

async function openServerPins() {
  if (!state.currentServer) return;
  closeFloating();
  $("pins-modal-title").textContent = `Pinned in ${state.currentServer.name}`;
  showModal("modal-pins");
  await renderServerPins();
}

async function renderServerPins() {
  const list = $("pins-modal-list");
  let pins = [];
  try {
    pins = await invoke("get_pins", { serverId: state.currentServer.id, channelId: null });
  } catch (e) {
    toast(String(e));
  }
  list.innerHTML = "";
  $("pins-modal-sub").textContent = pins.length
    ? `${pins.length} pinned ${pins.length === 1 ? "message" : "messages"} across this server, newest first.`
    : "Nothing is pinned in this server yet.";
  for (const group of groupPinsByChannel(pins)) {
    markPinsSeen(group.channel, group.pins);
    const head = el("div", "pins-group-head");
    const channel = state.currentServer.channels.find(c => c.id === group.channel);
    head.appendChild(channelMark(channel || { kind: "text" }, 16));
    head.appendChild(channel ? channelNameNode(channel) : document.createTextNode(group.channel_name));
    list.appendChild(head);
    for (const pin of group.pins) list.appendChild(pinRow(pin, { jump: () => jumpToPin(pin) }));
  }
}

const searchUi = { token: 0, timer: 0, items: [], index: -1, scope: null, place: null, offset: 0, page: null };
const SEARCH_DEBOUNCE_MS = 180;

function searchContext() {
  const server = state.currentServer;
  if (server) {
    return {
      members: server.members.map(m => ({ id: m.id, name: m.name })),
      channels: server.channels.filter(holdsMessages).map(c => ({ id: c.id, name: channelPlainName(c) })),
    };
  }
  if (state.view !== "dm") return { members: [], channels: [] };
  const members = (state.dmThreads || []).filter(t => t.kind !== "group").map(t => ({ id: t.user, name: t.name }));
  if (state.user) members.unshift({ id: state.user.id, name: state.user.username });
  return { members, channels: [] };
}

function searchScopeContext() {
  const server = state.currentServer;
  const group = Boolean(server && server.group);
  const dm = state.view === "dm";
  const channel = server && state.currentChannel ? state.currentChannel : null;
  const conversationName = dm ? state.currentDm && state.currentDm.name : group ? server.name : channel && channelPlainName(channel);
  return {
    view: state.view,
    group,
    conversation: dm ? Boolean(state.currentDm) : Boolean(channel),
    conversationName: conversationName || "",
    serverId: server ? server.id : null,
    channelId: channel ? channel.id : null,
    dmUser: dm && state.currentDm ? state.currentDm.user : null,
    active: dm || Boolean(server),
  };
}

function sameSearchPlace(a, b) {
  return a.view === b.view && a.serverId === b.serverId && a.dmUser === b.dmUser;
}

function currentSearchScope(ctx) {
  const place = searchPlace(ctx);
  if (searchUi.place !== place) {
    searchUi.place = place;
    searchUi.scope = defaultSearchScope(ctx);
    searchUi.offset = 0;
  } else {
    searchUi.scope = pickSearchScope(searchUi.scope, ctx);
  }
  return searchUi.scope;
}

function syncSearchBox() {
  const ctx = searchScopeContext();
  $("search-box").classList.toggle("hidden", !ctx.active);
  if (!ctx.active) closeSearchPop();
  currentSearchScope(ctx);
}

function renderSearchScope(ctx) {
  currentSearchScope(ctx);
  const buttons = searchScopes(ctx).map(scope => {
    const btn = el("button", "search-scope-btn", scope.label);
    btn.type = "button";
    btn.disabled = !scope.enabled;
    btn.classList.toggle("active", scope.key === searchUi.scope);
    btn.setAttribute("aria-pressed", scope.key === searchUi.scope ? "true" : "false");
    btn.addEventListener("mousedown", e => e.preventDefault());
    btn.onclick = () => setSearchScope(scope.key);
    return btn;
  });
  $("search-scope").replaceChildren(...buttons);
  return searchUi.scope;
}

function setSearchScope(key) {
  if (searchUi.scope === key) return;
  searchUi.scope = key;
  searchUi.offset = 0;
  renderSearchScope(searchScopeContext());
  if ($("search-input").value.trim()) runSearch(0);
  else renderSearchHints();
}

function setSearchPager(page) {
  searchUi.page = page;
  $("search-pager").classList.toggle("idle", !page);
  $("search-page-label").textContent = page && page.total > page.size ? pageLabel(page.total, page.offset) : "";
  $("search-older").disabled = !page || !page.hasOlder;
  $("search-newer").disabled = !page || !page.hasNewer;
}

function resetSearch() {
  clearTimeout(searchUi.timer);
  searchUi.token++;
  $("search-input").value = "";
  $("search-box").classList.remove("has-text");
  closeSearchPop();
}

function closeSearchPop() {
  $("search-pop").classList.add("hidden");
  $("search-box").classList.remove("open");
}

function openSearchPop() {
  const ctx = searchScopeContext();
  if (!ctx.active) return;
  closeFloating("search-pop");
  renderSearchScope(ctx);
  $("search-pop").classList.remove("hidden");
  $("search-box").classList.add("open");
}

function focusSearch() {
  if ($("search-box").classList.contains("hidden")) return;
  const input = $("search-input");
  input.focus();
  input.select();
}

function setSearchHead(title, sub) {
  $("search-pop-title").textContent = title;
  $("search-pop-sub").textContent = sub || "";
}

function setSearchItems(rows) {
  const list = $("search-list");
  list.replaceChildren(...rows);
  searchUi.items = rows.filter(r => r.dataset.pick === "1");
  searchUi.index = -1;
  list.scrollTop = 0;
}

function moveSearchSelection(step) {
  const items = searchUi.items;
  if (!items.length) return;
  searchUi.index = (searchUi.index + step + items.length) % items.length;
  items.forEach((row, i) => row.classList.toggle("active", i === searchUi.index));
  items[searchUi.index].scrollIntoView({ block: "nearest" });
}

function pickableRow(row, onPick) {
  row.dataset.pick = "1";
  row.setAttribute("role", "option");
  row.addEventListener("mousedown", e => e.preventDefault());
  row.onclick = onPick;
  return row;
}

function renderSearchHints() {
  const ctx = searchScopeContext();
  setSearchHead(searchScopeTitle(searchUi.scope, ctx), "Runs on this device, over the history it keeps");
  setSearchPager(null);
  const filters = SEARCH_FILTERS.filter(filter => filter.key !== "in" || searchContext().channels.length);
  const rows = filters.map(filter => {
    const row = el("button", "mention-row search-filter-row");
    row.type = "button";
    row.append(el("span", "search-filter-key", `${filter.key}:`), el("span", "mention-hint", filter.hint));
    return pickableRow(row, () => {
      const input = $("search-input");
      const next = applySearchFilter(input.value, filter.key);
      input.value = next.text;
      input.focus();
      input.setSelectionRange(next.caret, next.caret);
      onSearchInput();
    });
  });
  const tips = [["\"exact words\"", "words in this order"], ["@name", "same as mentions:"]].map(([key, hint]) => {
    const row = el("div", "mention-row search-tip-row");
    row.append(el("span", "search-filter-key", key), el("span", "mention-hint", hint));
    return row;
  });
  setSearchItems([el("div", "mention-head", "Filters"), ...rows, el("div", "mention-head search-tips-head", "Also works"), ...tips]);
}

function renderSearchSuggestions(token, items) {
  setSearchHead(token.kind === "member" ? (state.view === "dm" ? "People" : "Members") : "Channels", "");
  setSearchPager(null);
  const rows = items.map(item => {
    const row = el("button", "mention-row");
    row.type = "button";
    if (token.kind === "member") row.append(makeAvatar(item.id, item.name, "avatar avatar-sm"), el("span", "mention-name", item.name));
    else row.append(el("span", "mention-at", "#"), el("span", "mention-name", item.name));
    return pickableRow(row, () => {
      const input = $("search-input");
      const next = applySearchSuggestion(input.value, token, item);
      input.value = next.text;
      input.focus();
      input.setSelectionRange(next.caret, next.caret);
      onSearchInput();
    });
  });
  setSearchItems(rows);
  if (rows.length) moveSearchSelection(1);
}

function searchNote(title, text) {
  const empty = el("div", "pins-empty search-empty");
  const mark = el("div", "pins-empty-mark");
  mark.appendChild(svgIcon("search", 26));
  empty.append(mark, el("strong", null, title));
  if (text) empty.appendChild(el("span", null, text));
  return empty;
}

function highlightedExcerpt(message, terms) {
  const text = searchExcerpt(pinExcerpt(message, plainText), terms);
  const node = el("div", "pin-text search-text");
  if (!text) {
    node.classList.add("muted");
    return node;
  }
  for (const part of highlightParts(text, terms)) {
    node.appendChild(part.hit ? el("mark", "search-hit", part.text) : document.createTextNode(part.text));
  }
  return node;
}

function searchRow(message, terms, label, gated, onPick) {
  const row = el("div", "pin-row search-row");
  row.dataset.id = message.id;
  row.appendChild(makeAvatar(message.author_id, message.author_name, "avatar avatar-sm"));
  const body = el("div", "pin-body");
  const head = el("div", "pin-head");
  const author = el("span", "pin-author", message.author_name);
  decorateAuthor(author, message.author_id);
  head.append(author, el("span", "pin-time", formatTime(message.created_at)), el("span", "pin-channel", label));
  body.append(head, gated ? el("div", "pin-text search-text muted", SENSITIVE_PREVIEW) : highlightedExcerpt(message, terms));
  row.appendChild(body);
  return pickableRow(row, onPick);
}

function searchResultRow(hit, terms) {
  const channel = state.currentServer ? state.currentServer.channels.find(c => c.id === hit.channel) : null;
  const gated = sensitiveGated({ view: "server", channel, confirmed: sensitiveConfirmed() });
  return searchRow(hit.message, terms, `#${channel ? channelPlainName(channel) : hit.channel_name}`, gated, () => openSearchHit(hit));
}

function dmSearchResultRow(hit, terms, label = dmHitKey(hit).label) {
  return searchRow(searchHitMessage(hit), terms, label, false, () => openDmSearchHit(hit));
}

function searchTerms(query) {
  const terms = [...query.terms];
  if (query.mentions) terms.push(`@${query.mentions.name}`);
  return terms;
}

function renderSearchResults(query, results, scope, ctx) {
  const hits = results.hits.filter(h => !isBlocked(h.message.author_id));
  const total = results.total - (results.hits.length - hits.length);
  setSearchHead(pageCount(total), searchScopeLine(scope, query, ctx));
  setSearchPager(pageWindow(total, results.offset));
  if (!hits.length) {
    setSearchItems([searchNote("No messages found", "Search covers what this device keeps. Older history may have rolled off in Settings > Storage.")]);
    return;
  }
  setSearchItems(hits.map(hit => searchResultRow(hit, searchTerms(query))));
}

function renderDmSearchResults(query, results, scope, ctx) {
  const hits = results.hits.filter(h => !isBlocked(searchHitMessage(h).author_id));
  const total = results.total - (results.hits.length - hits.length);
  setSearchHead(pageCount(total), searchScopeLine(scope, query, ctx));
  setSearchPager(pageWindow(total, results.offset));
  if (!hits.length) {
    setSearchItems([searchNote("No messages found", "Search covers the direct messages and groups this device keeps.")]);
    return;
  }
  const terms = searchTerms(query);
  const runs = groupHits(hits);
  if (runs.length < 2) {
    setSearchItems(hits.map(hit => dmSearchResultRow(hit, terms)));
    return;
  }
  setSearchItems(runs.flatMap(run => [el("div", "mention-head search-run-head", run.label), ...run.hits.map(hit => dmSearchResultRow(hit, terms, ""))]));
}

async function runSearch(offset = searchUi.offset) {
  clearTimeout(searchUi.timer);
  const ctx = searchScopeContext();
  if (!ctx.active) return;
  const scope = renderSearchScope(ctx);
  const query = parseSearch($("search-input").value, searchContext());
  const token = ++searchUi.token;
  if (query.unknown.length) {
    setSearchHead(searchScopeTitle(scope, ctx), "");
    setSearchPager(null);
    const where = ctx.view === "dm" ? "in your direct messages" : "in this server";
    setSearchItems([searchNote(unknownFilterText(query.unknown, where), "Pick a name from the list as you type.")]);
    return;
  }
  if (searchIsEmpty(query)) return renderSearchHints();
  const request = searchRequest(scope, searchArgs(query), ctx, offset);
  let results;
  try {
    results = await invoke(request.command, request.args);
  } catch (e) {
    if (token === searchUi.token) {
      setSearchPager(null);
      setSearchItems([searchNote("Search did not finish", String(e))]);
    }
    return;
  }
  if (token !== searchUi.token || !sameSearchPlace(ctx, searchScopeContext())) return;
  searchUi.offset = results.offset || 0;
  if (request.command === "search_dms") renderDmSearchResults(query, results, scope, ctx);
  else renderSearchResults(query, results, scope, ctx);
}

function turnSearchPage(direction) {
  const page = searchUi.page;
  if (!page) return;
  const next = direction > 0 ? page.olderOffset : page.newerOffset;
  if (next !== page.offset) runSearch(next);
}

function onSearchInput() {
  const input = $("search-input");
  $("search-box").classList.toggle("has-text", input.value.length > 0);
  searchUi.offset = 0;
  openSearchPop();
  const token = searchToken(input.value, input.selectionStart ?? input.value.length);
  const items = searchSuggestions(token, searchContext());
  clearTimeout(searchUi.timer);
  if (token && items.length) {
    searchUi.token++;
    renderSearchSuggestions(token, items);
    return;
  }
  if (!input.value.trim()) {
    searchUi.token++;
    renderSearchHints();
    return;
  }
  searchUi.timer = setTimeout(runSearch, SEARCH_DEBOUNCE_MS);
}

function onSearchKeydown(e) {
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if ($("search-pop").classList.contains("hidden")) onSearchInput();
    moveSearchSelection(e.key === "ArrowDown" ? 1 : -1);
  } else if (e.key === "Enter" || (e.key === "Tab" && searchUi.index >= 0 && searchUi.items[searchUi.index].classList.contains("mention-row"))) {
    e.preventDefault();
    const picked = searchUi.items[searchUi.index];
    if (picked) picked.click();
    else {
      openSearchPop();
      runSearch();
    }
  } else if ((e.key === "PageDown" || e.key === "PageUp") && searchUi.page) {
    e.preventDefault();
    turnSearchPage(e.key === "PageDown" ? 1 : -1);
  } else if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    if (!$("search-pop").classList.contains("hidden")) closeSearchPop();
    else e.target.blur();
  }
}

async function openSearchHit(hit) {
  closeSearchPop();
  $("search-input").blur();
  if (!state.currentServer) return;
  const channel = state.currentServer.channels.find(c => c.id === hit.channel);
  if (!channel) {
    toast("That channel is gone");
    return;
  }
  if (!state.currentChannel || state.currentChannel.id !== channel.id) await selectChannel(channel);
  await jumpToMessage(hit.message.id);
}

async function openDmSearchHit(hit) {
  closeSearchPop();
  $("search-input").blur();
  try {
    if (hit.kind === "group") {
      await openMessageLink({ server: hit.server, channel: hit.channel, message: hit.message.id });
      return;
    }
    if (state.view !== "dm") {
      setViewMode("dm");
      state.currentServer = null;
      state.currentChannel = null;
      renderServerRail();
    }
    if (!state.currentDm || state.currentDm.user !== hit.user) await selectDm(hit.user, hit.name);
    await jumpToMessage(hit.message.id);
  } catch (e) {
    toast(String(e));
  }
}

function wireSearch() {
  const input = $("search-input");
  $("search-box-icon").appendChild(svgIcon("search", 16));
  setIcon($("search-clear"), "close", 14);
  input.addEventListener("input", onSearchInput);
  input.addEventListener("focus", onSearchInput);
  input.addEventListener("keydown", onSearchKeydown);
  $("search-clear").addEventListener("mousedown", e => e.preventDefault());
  $("search-clear").onclick = () => {
    input.value = "";
    input.focus();
    onSearchInput();
  };
  for (const [id, direction] of [["search-older", 1], ["search-newer", -1]]) {
    $(id).addEventListener("mousedown", e => e.preventDefault());
    $(id).onclick = () => turnSearchPage(direction);
  }
  document.addEventListener("keydown", e => {
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "f" && $("modal-backdrop").classList.contains("hidden")) {
      if ($("search-box").classList.contains("hidden")) return;
      e.preventDefault();
      focusSearch();
    }
  });
}

const SWITCHER_RECENT_KEY = "commy.switcher.recent";
const SWITCHER_MOD = /mac/i.test(navigator.platform || "") ? "Cmd" : "Ctrl";
const switcher = { items: [], shown: [], at: 0, views: {}, open: false, recent: [] };

function loadSwitcherRecent() {
  try {
    return readRecent(localStorage.getItem(SWITCHER_RECENT_KEY));
  } catch {
    return [];
  }
}

function rememberSwitcherRecent(key) {
  switcher.recent = rememberRecent(loadSwitcherRecent(), key);
  try { localStorage.setItem(SWITCHER_RECENT_KEY, JSON.stringify(switcher.recent)); } catch {}
}

function switcherActions() {
  return [
    { id: "settings", name: "Settings", subtitle: "Account, voice, appearance" },
    { id: "create-server", name: "Create a server", subtitle: "From scratch or a template" },
  ];
}

function switcherPresence(userId) {
  return PRESENCE_LABELS[personPresence(userId)] || "";
}

async function refreshSwitcherViews() {
  const views = {};
  for (const summary of state.servers) {
    try {
      views[summary.id] = await invoke("get_server", { serverId: summary.id });
    } catch {
      continue;
    }
  }
  switcher.views = views;
}

function rebuildSwitcherItems() {
  switcher.items = buildSwitcherItems({
    servers: state.servers, views: switcher.views, threads: state.dmThreads || [], friends: friends.friends,
    presence: switcherPresence, plainName: channelPlainName, actions: switcherActions(),
  });
}

async function openSwitcher() {
  if (!state.user) return;
  const input = $("switcher-input");
  switcher.open = true;
  switcher.at = 0;
  switcher.recent = loadSwitcherRecent();
  $("switcher-backdrop").classList.remove("hidden");
  input.value = "";
  rebuildSwitcherItems();
  renderSwitcher();
  input.focus();
  await refreshSwitcherViews();
  if (!switcher.open) return;
  try {
    state.dmThreads = await invoke("list_dm_threads");
  } catch {}
  if (!switcher.open) return;
  rebuildSwitcherItems();
  renderSwitcher();
}

function closeSwitcher() {
  switcher.open = false;
  $("switcher-backdrop").classList.add("hidden");
}

function renderSwitcher() {
  const list = $("switcher-list");
  switcher.shown = rankSwitcher(switcher.items, $("switcher-input").value, switcher.recent);
  switcher.at = Math.min(switcher.at, Math.max(0, switcher.shown.length - 1));
  list.innerHTML = "";
  if (!switcher.shown.length) {
    list.appendChild(el("div", "switcher-empty", switcher.items.length ? "Nothing matches" : "Join or create a server first"));
    return;
  }
  switcher.shown.forEach((item, at) => {
    const row = el("div", "switcher-row");
    row.setAttribute("role", "option");
    row.appendChild(switcherGlyph(item));
    const text = el("div", "switcher-text");
    text.appendChild(el("span", "switcher-name", item.name));
    if (item.subtitle) text.appendChild(el("span", "switcher-sub", item.subtitle));
    row.appendChild(text);
    row.onmousemove = () => {
      if (switcher.at !== at) highlightSwitcher(at, false);
    };
    row.onmousedown = e => e.preventDefault();
    row.onclick = () => runSwitcherItem(item);
    list.appendChild(row);
  });
  highlightSwitcher(switcher.at, true);
}

function switcherGlyph(item) {
  if (item.kind === "person" && !item.group) return makeAvatar(item.userId, item.name, "avatar switcher-avatar");
  const glyph = el("div", "switcher-glyph");
  if (item.kind === "server") {
    const summary = state.servers.find(s => s.id === item.serverId);
    glyph.classList.add("switcher-server-icon");
    if (summary) paintServerIcon(glyph, summary);
    return glyph;
  }
  if (item.kind === "person") glyph.appendChild(svgIcon("people", 18));
  else if (item.kind === "action") glyph.appendChild(svgIcon(item.action === "settings" ? "gear" : "plus", 18));
  else if (isVoiceKind(item.channelKind)) glyph.appendChild(voiceKindIcon(item.channelKind, 18));
  else if (item.channelKind === "announcement") glyph.appendChild(svgIcon("megaphone", 18));
  else glyph.textContent = "#";
  return glyph;
}

function highlightSwitcher(at, scroll) {
  const rows = $("switcher-list").querySelectorAll(".switcher-row");
  if (!rows.length) return;
  switcher.at = (at + rows.length) % rows.length;
  rows.forEach((row, i) => {
    row.classList.toggle("active", i === switcher.at);
    row.setAttribute("aria-selected", i === switcher.at ? "true" : "false");
  });
  if (scroll) rows[switcher.at].scrollIntoView({ block: "nearest" });
}

async function runSwitcherItem(item) {
  closeSwitcher();
  if (openSettingsLayerId()) await closeSettingsLayers();
  if (item.kind === "action") {
    if (item.action === "settings") openUserSettings("voice");
    else await openCreateServer();
    return;
  }
  if (item.kind === "server" || item.group) return selectServer(item.serverId);
  if (item.kind === "person") {
    const known = (state.dmThreads || []).some(t => t.user === item.userId);
    return known ? selectDm(item.userId, item.name) : messageUser(item.userId, item.name);
  }
  if (!state.currentServer || state.currentServer.id !== item.serverId) await selectServer(item.serverId);
  const channel = state.currentServer && state.currentServer.channels.find(c => c.id === item.channelId);
  if (!channel) return toast("That channel is gone");
  if (isVoiceKind(channel.kind)) {
    if (state.voiceChannelId !== channel.id) await toggleVoice(channel);
    return;
  }
  await selectChannel(channel);
}

function wireSwitcher() {
  const input = $("switcher-input");
  $("switcher-icon").appendChild(svgIcon("search", 18));
  for (const node of document.querySelectorAll("[data-shortcut]")) node.textContent = `${SWITCHER_MOD}+${node.dataset.shortcut}`;
  input.addEventListener("input", () => {
    switcher.at = 0;
    renderSwitcher();
  });
  input.addEventListener("keydown", e => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      highlightSwitcher(switcher.at + 1, true);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      highlightSwitcher(switcher.at - 1, true);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = switcher.shown[switcher.at];
      if (item) runSwitcherItem(item);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeSwitcher();
    }
  });
  $("switcher-backdrop").addEventListener("mousedown", e => {
    if (e.target === e.currentTarget) closeSwitcher();
  });
  document.addEventListener("keydown", e => {
    if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey || e.key.toLowerCase() !== "k") return;
    if (!state.user) return;
    e.preventDefault();
    if (switcher.open) closeSwitcher();
    else if ($("modal-backdrop").classList.contains("hidden")) openSwitcher();
  });
}

function replyReference(message) {
  const ref = el("div", "reply-ref");
  const preview = message.reply_preview;
  if (preview) {
    ref.appendChild(makeAvatar(preview.author_id, preview.author_name, "avatar avatar-xs"));
    ref.appendChild(el("span", "reply-ref-name", preview.author_name));
    ref.appendChild(el("span", "reply-ref-text", plainText(preview.excerpt)));
  } else {
    ref.appendChild(el("span", "reply-ref-text missing", "Original message was deleted"));
  }
  ref.onclick = () => jumpToMessage(message.reply_to);
  return ref;
}

function messageRow(id) {
  return $("messages").querySelector(`.msg[data-id="${CSS.escape(id)}"]`);
}

async function fetchChannelMessages(channel) {
  const anchored = state.historyAnchor && state.historyAnchor.channel === channel.id ? state.historyAnchor.message : null;
  if (!anchored) state.historyAnchor = null;
  const args = { serverId: state.currentServer.id, channelId: channel.id };
  if (!anchored) return invoke("get_messages", args);
  try {
    return await invoke("get_messages", { ...args, around: anchored });
  } catch {
    state.historyAnchor = null;
    return invoke("get_messages", args);
  }
}

async function showMessageInHistory(id) {
  const channel = state.currentChannel;
  if (state.view !== "server" || !channel || !state.currentServer) return null;
  if (state.historyAnchor) {
    state.historyAnchor = null;
    $("messages").dataset.view = "";
    renderMessages(await fetchChannelMessages(channel));
    const row = messageRow(id);
    if (row) return row;
  }
  state.historyAnchor = { channel: channel.id, message: id };
  const messages = await fetchChannelMessages(channel);
  if (state.currentChannel !== channel) return null;
  renderMessages(messages);
  return messageRow(id);
}

function historyPill() {
  const pill = el("div", "history-pill");
  pill.setAttribute("role", "status");
  pill.appendChild(el("span", null, "Viewing older messages"));
  const back = el("button", "btn-primary btn-small", "Jump to present");
  back.onclick = jumpToPresent;
  pill.appendChild(back);
  return pill;
}

async function jumpToPresent() {
  state.historyAnchor = null;
  if (state.view !== "server" || !state.currentChannel) return;
  $("messages").dataset.view = "";
  renderMessages(await fetchChannelMessages(state.currentChannel));
}

async function jumpToMessage(id) {
  let row = messageRow(id);
  if (!row) row = await showMessageInHistory(id).catch(() => null);
  if (!row) {
    toast("That message is no longer on this device");
    return;
  }
  phoneShowChat();
  row.scrollIntoView({ behavior: "smooth", block: "center" });
  row.classList.remove("flash");
  void row.offsetWidth;
  row.classList.add("flash");
}

function startReply(message) {
  state.replyingTo = message;
  $("reply-bar-name").textContent = message.author_name;
  $("reply-bar").classList.remove("hidden");
  $("message-input").focus();
}

function cancelReply() {
  state.replyingTo = null;
  $("reply-bar").classList.add("hidden");
}

let threadRenaming = null;
let threadsArchivedOpen = false;

function currentThreads() {
  return (state.view === "server" && state.currentChannel && state.currentChannel.threads) || [];
}

function threadMetaOf(rootId) {
  return currentThreads().find(t => t.root === rootId) || null;
}

function canManageMessagesHere() {
  return Boolean(state.currentServer && state.currentServer.can_manage_messages);
}

function canManageCurrentThread(root) {
  const meta = root ? threadMetaOf(root.id) : null;
  return canManageThread(root, {
    userId: state.user && state.user.id,
    canManageMessages: canManageMessagesHere(),
    meta,
    canCreateThreads: canCreateThreads(state.currentServer),
  });
}

function openThread(rootId) {
  state.threadRoot = rootId;
  $("app").classList.add("thread-open");
  $("thread-panel").classList.remove("hidden");
  renderThread();
  if (!$("thread-input").disabled) $("thread-input").focus();
}

function closeThread() {
  state.threadRoot = null;
  cancelThreadRename();
  $("app").classList.remove("thread-open");
  $("thread-panel").classList.add("hidden");
}

function renderThreadHead(root, replies) {
  const meta = root ? threadMetaOf(root.id) : null;
  const manages = canManageCurrentThread(root);
  const renaming = Boolean(root) && threadRenaming === root.id && manages;
  if (!renaming) threadRenaming = null;
  const title = $("thread-title");
  const post = root ? forumPostFor(root.id) : null;
  title.textContent = post ? post.title : root ? threadName(root, meta, plainText) : "Thread";
  title.disabled = !manages;
  title.title = manages ? "Rename Thread" : "";
  title.classList.toggle("hidden", renaming);
  $("thread-title-input").classList.toggle("hidden", !renaming);
  $("thread-status").textContent = root ? threadStatusLine({ meta, replies }) : "";
  for (const id of ["thread-rename", "thread-archive", "thread-lock"]) $(id).classList.toggle("hidden", !manages);
  const flag = (id, on, onTitle, offTitle) => {
    const btn = $(id);
    btn.classList.toggle("active", on);
    btn.setAttribute("aria-pressed", String(on));
    btn.title = on ? onTitle : offTitle;
  };
  flag("thread-archive", Boolean(meta && meta.archived), "Unarchive Thread", "Archive Thread");
  flag("thread-lock", Boolean(meta && meta.locked), "Unlock Thread", "Lock Thread");
  const composer = threadComposerState({ meta, canManageMessages: canManageMessagesHere() });
  const input = $("thread-input");
  input.disabled = composer.disabled;
  input.placeholder = composer.placeholder;
  input.closest(".composer").classList.toggle("locked", composer.disabled);
}

function startThreadRename() {
  const root = (state.renderedMessages || []).find(m => m.id === state.threadRoot);
  if (!root || !canManageCurrentThread(root)) return;
  threadRenaming = root.id;
  const meta = threadMetaOf(root.id);
  const input = $("thread-title-input");
  input.value = meta && meta.name ? meta.name : "";
  input.placeholder = threadName(root, null, plainText);
  $("thread-title").classList.add("hidden");
  input.classList.remove("hidden");
  input.focus();
  input.select();
}

function cancelThreadRename() {
  threadRenaming = null;
  $("thread-title-input").classList.add("hidden");
  $("thread-title").classList.remove("hidden");
}

async function commitThreadRename() {
  const rootId = threadRenaming;
  if (!rootId) return;
  const { name, error } = cleanThreadName($("thread-title-input").value);
  cancelThreadRename();
  if (error) return toast(error);
  const meta = threadMetaOf(rootId);
  if ((meta ? meta.name : null) === name) return;
  await setThread(rootId, { name, archived: Boolean(meta && meta.archived), locked: Boolean(meta && meta.locked) });
}

async function setThread(rootId, { name, archived, locked }) {
  if (!state.currentServer || !state.currentChannel) return;
  try {
    await invoke("set_thread", { serverId: state.currentServer.id, channelId: state.currentChannel.id, rootId, name, archived, locked });
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

async function toggleThreadFlag(flag) {
  const rootId = state.threadRoot;
  if (!rootId) return;
  const meta = threadMetaOf(rootId) || { name: null, archived: false, locked: false };
  await setThread(rootId, {
    name: meta.name,
    archived: flag === "archived" ? !meta.archived : meta.archived,
    locked: flag === "locked" ? !meta.locked : meta.locked,
  });
}

function syncThreadsButton(messages = state.renderedMessages || []) {
  const btn = $("threads-btn");
  const channel = state.view === "server" ? state.currentChannel : null;
  const { active } = buildThreadList(channel ? messages : [], currentThreads(), { plain: plainText });
  const { visible, title } = threadsButtonState({ view: state.view, channel, count: active.length });
  btn.classList.toggle("hidden", !visible);
  btn.title = title;
  if (!visible) return closeThreadsPanel();
  if (!$("threads-pop").classList.contains("hidden")) renderThreadsPanel();
}

function closeThreadsPanel() {
  $("threads-pop").classList.add("hidden");
  $("threads-btn").classList.remove("active");
}

function toggleThreadsPanel() {
  const pop = $("threads-pop");
  if (!pop.classList.contains("hidden")) return closeThreadsPanel();
  closeFloating();
  pop.classList.remove("hidden");
  $("threads-btn").classList.add("active");
  renderThreadsPanel();
}

function renderThreadsPanel() {
  const list = $("threads-list");
  list.innerHTML = "";
  if (!state.currentChannel) return;
  const { active, archived } = buildThreadList(state.renderedMessages || [], currentThreads(), { plain: plainText });
  const where = `#${channelPlainName(state.currentChannel)}`;
  $("threads-pop-sub").textContent = active.length ? `${active.length} active in ${where}` : `None active in ${where}`;
  if (!active.length && !archived.length) {
    list.appendChild(threadsEmptyState());
    return;
  }
  for (const thread of active) list.appendChild(threadRow(thread));
  if (!archived.length) return;
  const toggle = el("button", "threads-archived-toggle");
  toggle.type = "button";
  toggle.setAttribute("aria-expanded", String(threadsArchivedOpen));
  toggle.appendChild(svgIcon(threadsArchivedOpen ? "chevronDown" : "chevronRight", 16));
  toggle.appendChild(el("span", null, `Archived (${archived.length})`));
  toggle.onclick = () => {
    threadsArchivedOpen = !threadsArchivedOpen;
    renderThreadsPanel();
  };
  list.appendChild(toggle);
  if (threadsArchivedOpen) for (const thread of archived) list.appendChild(threadRow(thread));
}

function threadsEmptyState() {
  const empty = el("div", "pins-empty");
  const mark = el("div", "pins-empty-mark");
  mark.appendChild(svgIcon("thread", 26));
  empty.appendChild(mark);
  empty.appendChild(el("strong", null, "No threads yet"));
  empty.appendChild(el("span", null, "Hover a message and choose Open Thread to start one."));
  return empty;
}

function threadRow(thread) {
  const row = el("div", "thread-row" + (thread.archived ? " archived" : ""));
  row.dataset.id = thread.id;
  row.setAttribute("role", "button");
  row.tabIndex = 0;
  const tile = el("span", "thread-row-icon");
  tile.appendChild(svgIcon(thread.locked ? "lock" : "thread", 16));
  row.appendChild(tile);
  const body = el("div", "thread-row-body");
  body.appendChild(el("div", "thread-row-name", thread.name));
  const meta = [`${thread.replies} ${thread.replies === 1 ? "reply" : "replies"}`];
  if (thread.locked) meta.push("Locked");
  meta.push(formatShortTime(new Date(thread.lastAt).toISOString()));
  body.appendChild(el("div", "thread-row-meta", meta.join(" · ")));
  row.appendChild(body);
  const open = () => {
    closeThreadsPanel();
    openThread(thread.id);
  };
  row.onclick = open;
  row.onkeydown = e => {
    if (e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    open();
  };
  return row;
}

function renderThread() {
  const messages = state.renderedMessages || [];
  const byId = new Map(messages.map(m => [m.id, m]));
  const root = byId.get(state.threadRoot);
  const box = $("thread-messages");
  box.innerHTML = "";
  const items = root ? messages.filter(m => m.id === root.id || (m.reply_to && threadRootOf(m, byId) === root.id)) : [];
  renderThreadHead(root, Math.max(0, items.length - 1));
  if (!root) {
    box.appendChild(el("div", "manage-empty", "This thread's first message is no longer loaded."));
    return;
  }
  items.forEach((m, i) => {
    const row = buildMessageRow(m, false, { inThread: true });
    if (i === 0) row.classList.add("thread-root");
    box.appendChild(row);
    if (i === 0) {
      const n = items.length - 1;
      box.appendChild(el("div", "thread-divider", n ? `${n} ${n === 1 ? "reply" : "replies"}` : "No replies yet"));
    }
  });
  box.scrollTop = box.scrollHeight;
}

async function sendThreadReply() {
  const input = $("thread-input");
  const content = input.value.trim();
  if (!content || !state.threadRoot || !state.currentChannel) return;
  input.value = "";
  fitComposer(input);
  try {
    await invoke("send_message", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      content: encodeOutgoing(content),
      replyTo: state.threadRoot,
      previewUrl: await previewUrlFor(content),
    });
    await refreshCurrentServer();
  } catch (e) {
    input.value = content;
    fitComposer(input);
    toast(String(e));
  }
}

function reactionPills(message) {
  const reactions = message.reactions || [];
  if (!reactions.length || !(state.view === "server" || inDmView())) return null;
  const wrap = el("div", "reactions");
  for (const r of reactions) {
    const mine = Boolean(state.user && r.users.includes(state.user.id));
    const pill = el("button", "reaction" + (mine ? " mine" : ""));
    const custom = parseEmojiToken(r.emoji);
    if (custom) pill.appendChild(customEmojiImg(custom.name, custom.image, "custom-emoji reaction-emoji"));
    else pill.appendChild(el("span", "reaction-emoji", r.emoji));
    pill.appendChild(el("span", "reaction-count", String(r.count)));
    const names = r.users.map(id => (state.user && id === state.user.id ? "You" : avatarNameFor(id))).join(", ");
    pill.title = `${names} reacted with ${custom ? `:${custom.name}:` : r.emoji}`;
    pill.onclick = () => setReaction(message, r.emoji, !mine);
    wrap.appendChild(pill);
  }
  const add = el("button", "reaction add");
  add.title = "Add Reaction";
  add.appendChild(svgIcon("smilePlus", 16));
  add.onclick = e => openEmojiPop(e.currentTarget, emoji => setReaction(message, emoji, true), { custom: true });
  wrap.appendChild(add);
  return wrap;
}

let currentMembersSource = null;
let currentMembersById = new Map();

function currentServerMember(id) {
  const server = state.currentServer;
  if (!server) return server;
  if (currentMembersSource !== server.members) {
    const byId = new Map();
    for (const m of server.members) if (!byId.has(m.id)) byId.set(m.id, m);
    currentMembersSource = server.members;
    currentMembersById = byId;
  }
  return currentMembersById.get(id);
}

function avatarNameFor(userId) {
  if (inDmView() && userId === state.currentDm.user) return state.currentDm.name;
  const member = currentServerMember(userId);
  return member ? member.name : "Someone";
}

async function setReaction(message, emoji, active) {
  if (inDmView()) {
    try {
      await invoke("set_dm_reaction", { userId: state.currentDm.user, messageId: message.id, emoji, active });
      const box = $("messages");
      const keep = box.scrollTop;
      await selectDm(state.currentDm.user, state.currentDm.name);
      box.scrollTop = keep;
    } catch (e) {
      toast(String(e));
    }
    return;
  }
  if (!state.currentServer || !state.currentChannel) return;
  try {
    await invoke("set_reaction", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      messageId: message.id,
      emoji,
      active,
    });
    const box = $("messages");
    const keep = box.scrollTop;
    await selectChannel(state.currentChannel);
    box.scrollTop = keep;
  } catch (e) {
    toast(String(e));
  }
}

const HIDDEN_KEY = "commy.hiddenMessages";
let hiddenMessages = null;

function hiddenSet() {
  if (!hiddenMessages) {
    try {
      hiddenMessages = new Set(JSON.parse(localStorage.getItem(HIDDEN_KEY) || "[]"));
    } catch {
      hiddenMessages = new Set();
    }
  }
  return hiddenMessages;
}

function isHidden(id) {
  return hiddenSet().has(id);
}

function setHidden(id, hidden) {
  const set = hiddenSet();
  if (hidden) set.add(id);
  else set.delete(id);
  try {
    const MAX_REMEMBERED_HIDES = 2000;
    localStorage.setItem(HIDDEN_KEY, JSON.stringify([...set].slice(-MAX_REMEMBERED_HIDES)));
  } catch {}
  renderMessages(state.renderedMessages || []);
}

function hiddenPlaceholder(message) {
  const row = el("div", "msg-hidden");
  row.appendChild(svgIcon("eyeOff", 14));
  row.appendChild(el("span", null, `You hid a message from ${message.author_name}`));
  const show = el("button", "btn-link", "Show");
  show.onclick = () => setHidden(message.id, false);
  row.appendChild(show);
  return row;
}

function isOwnMessage(message) {
  return Boolean(state.user && message.author_id === state.user.id);
}

function canDeleteMessage(message) {
  if (state.view === "dm") return Boolean(state.currentDm) && isOwnMessage(message);
  if (state.view !== "server" || !state.currentServer) return false;
  return isOwnMessage(message) || Boolean(state.currentServer.can_manage_messages);
}

function inDmView() {
  return state.view === "dm" && Boolean(state.currentDm);
}

function isGroupView() {
  return state.view === "server" && Boolean(state.currentServer && state.currentServer.group);
}

function dmListShown() {
  return state.view === "dm" || isGroupView();
}

function messageActions(message, inThread = false, replies = 0) {
  const bar = el("div", "msg-actions");
  const add = (icon, title, onClick, cls = "") => {
    const btn = el("button", cls);
    btn.title = title;
    btn.appendChild(svgIcon(icon, 18));
    btn.onclick = e => {
      e.stopPropagation();
      onClick(e);
    };
    bar.appendChild(btn);
  };
  if (state.view === "server" || inDmView()) {
    add("smilePlus", "Add Reaction", e => openEmojiPop(e.currentTarget, emoji => setReaction(message, emoji, true), { custom: true }));
    if (inThread) {
      add("reply", "Reply", () => $("thread-input").focus());
    } else {
      add("reply", "Reply", () => startReply(message));
      if (state.view === "server" && threadEntryShown({ replies, meta: threadMetaOf(message.id), canCreate: canCreateThreads(state.currentServer) })) {
        add("thread", "Open Thread", () => openThread(message.id));
      }
    }
  }
  if (state.view === "server" && canPinIn(state.currentServer, state.currentChannel)) {
    if (message.pinned) add("pinOff", "Unpin", () => setPinned(message, false));
    else add("pin", "Pin", () => setPinned(message, true));
  }
  if ((state.view === "server" || inDmView()) && isOwnMessage(message) && message.content && !parseStickerToken(message.content)) {
    add("pencil", "Edit", () => startEditMessage(message.id));
  }
  if (!isOwnMessage(message)) add("eyeOff", "Hide for Me", () => setHidden(message.id, true));
  savedMessageActions(add, message);
  if (message.content && !parseStickerToken(message.content)) add("more", "More", e => openMessageMenu(message, e.currentTarget));
  if (state.view === "server" && reportableIn(state.currentServer, message.author_id)) {
    add("flag", "Report Message", () => openReport({
      serverId: state.currentServer.id,
      userId: message.author_id,
      name: message.author_name,
      channelId: state.currentChannel && state.currentChannel.id,
      messageId: message.id,
      excerpt: message.content,
    }));
  }
  if (canDeleteMessage(message)) {
    add("trash", isOwnMessage(message) ? "Delete" : "Delete (Moderator)", e => {
      if (e.shiftKey) deleteMessage(message);
      else openDeleteMessage(message);
    }, "danger");
  }
  return bar;
}

function startEditMessage(id) {
  const message = (state.renderedMessages || []).find(m => m.id === id);
  const row = $("messages").querySelector(`.msg[data-id="${CSS.escape(id)}"]`);
  if (!message || !row || row.classList.contains("editing")) return;
  row.classList.add("editing");
  const body = row.querySelector(".msg-body");
  const content = body.querySelector(".msg-content");
  const editor = el("div", "msg-editor");
  const input = el("textarea");
  input.value = decodeRoleMentions(decodeCustomEmojis(message.content), currentRoles());
  input.rows = Math.min(8, input.value.split("\n").length);
  editor.appendChild(input);
  const hint = el("div", "msg-editor-hint");
  hint.innerHTML = "escape to <a>cancel</a> &middot; enter to <a>save</a>";
  const [cancelLink, saveLink] = hint.querySelectorAll("a");
  editor.appendChild(hint);
  if (content) content.replaceWith(editor);
  else body.appendChild(editor);

  const finish = () => renderMessages(state.renderedMessages || []);
  const save = async () => {
    const text = input.value.trim();
    if (!text) {
      openDeleteMessage(message);
      return;
    }
    if (encodeOutgoing(text) === message.content) return finish();
    try {
      if (inDmView()) {
        await invoke("edit_dm", { userId: state.currentDm.user, messageId: message.id, content: encodeCustomEmojis(text) });
        await selectDm(state.currentDm.user, state.currentDm.name);
        return;
      }
      await invoke("edit_message", {
        serverId: state.currentServer.id,
        channelId: state.currentChannel.id,
        messageId: message.id,
        content: encodeOutgoing(text),
      });
      await selectChannel(state.currentChannel);
    } catch (e) {
      toast(String(e));
    }
  };
  cancelLink.onclick = finish;
  saveLink.onclick = save;
  input.addEventListener("keydown", e => {
    if (e.key === "Escape") {
      e.stopPropagation();
      finish();
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      save();
    }
  });
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
}

let pendingDelete = null;

function openDeleteMessage(message) {
  pendingDelete = message;
  const preview = $("delete-message-preview");
  preview.replaceChildren();
  preview.appendChild(makeAvatar(message.author_id, message.author_name));
  const body = el("div", "msg-body");
  const head = el("div", "msg-head");
  head.appendChild(el("span", "msg-author", message.author_name));
  head.appendChild(el("span", "msg-time", formatTime(message.created_at)));
  body.appendChild(head);
  body.appendChild(el("div", "msg-content", message.content || (message.attachments || []).map(a => a.filename).join(", ")));
  preview.appendChild(body);
  $("delete-message-mod").classList.toggle("hidden", isOwnMessage(message));
  showModal("modal-delete-message");
}

async function deleteMessage(message) {
  try {
    if (inDmView()) {
      await invoke("delete_dm", { userId: state.currentDm.user, messageId: message.id });
      hideModals();
      await selectDm(state.currentDm.user, state.currentDm.name);
      return;
    }
    await invoke("delete_message", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      messageId: message.id,
    });
    hideModals();
    await selectChannel(state.currentChannel);
  } catch (e) {
    toast(String(e));
  }
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

const voiceClips = new Map();
let voicePlaying = null;

function voicePlaybackSupported() {
  if (typeof Audio === "undefined") return false;
  return new Audio().canPlayType(VOICE_PLAYBACK_TYPE) !== "";
}

async function loadVoiceClip(att) {
  if (voiceClips.has(att.blob)) return voiceClips.get(att.blob);
  const bytes = await invoke("get_voice_message", { blob: att.blob, provider: att.provider ?? null, sizeBytes: att.size_bytes ?? null, serverId: att.server ?? null });
  const clip = { url: URL.createObjectURL(new Blob([bytes], { type: VOICE_PLAYBACK_TYPE })), duration: oggDuration(new Uint8Array(bytes)) };
  voiceClips.set(att.blob, clip);
  return clip;
}

function renderVoiceMessage(att) {
  const box = el("div", "voice-msg");
  box.title = "Voice message";
  const play = el("button", "voice-play");
  play.type = "button";
  play.title = "Play";
  play.appendChild(svgIcon("play", 16));
  const wave = el("div", "voice-wave");
  const bars = waveformBars(att.blob).map(height => {
    const bar = el("i");
    bar.style.height = `${Math.round(height * 100)}%`;
    wave.appendChild(bar);
    return bar;
  });
  const elapsed = el("span", "voice-elapsed", "0:00");
  const total = el("span", "voice-total", "–:––");
  const time = el("span", "voice-time");
  time.append(elapsed, el("span", "voice-sep", "/"), total);
  box.append(play, wave, time);
  let audio = null;
  let duration = null;
  const paint = progress => {
    const played = playedBars(progress, bars.length);
    bars.forEach((bar, i) => bar.classList.toggle("played", i < played));
  };
  const showPlaying = playing => {
    play.replaceChildren(svgIcon(playing ? "pause" : "play", 16));
    play.title = playing ? "Pause" : "Play";
    box.classList.toggle("playing", playing);
  };
  loadVoiceClip(att).then(clip => {
    duration = clip.duration;
    if (duration !== null) total.textContent = formatClock(duration);
  }).catch(() => {});
  play.onclick = async () => {
    if (audio && !audio.paused) {
      audio.pause();
      return;
    }
    if (!audio) {
      play.disabled = true;
      try {
        const clip = await loadVoiceClip(att);
        audio = new Audio(clip.url);
        audio.preload = "auto";
        audio.ontimeupdate = () => {
          const length = duration || audio.duration || 0;
          elapsed.textContent = formatClock(audio.currentTime);
          paint(length > 0 ? audio.currentTime / length : 0);
        };
        audio.onplay = () => {
          if (voicePlaying && voicePlaying !== audio) voicePlaying.pause();
          voicePlaying = audio;
          showPlaying(true);
        };
        audio.onpause = () => showPlaying(false);
        audio.onended = () => {
          showPlaying(false);
          elapsed.textContent = "0:00";
          paint(0);
        };
        audio.onerror = () => {
          showPlaying(false);
          toast("This voice message could not be played");
        };
      } catch (e) {
        toast(String(e));
        play.disabled = false;
        return;
      }
      play.disabled = false;
    }
    audio.play().catch(() => toast("This voice message could not be played"));
  };
  return box;
}

function attachmentHere(att) {
  return state.view === "server" && state.currentServer ? { ...att, server: state.currentServer.id } : att;
}

function renderAttachment(att) {
  if (isVoiceMessage(att) && voicePlaybackSupported()) return renderVoiceMessage(att);
  const box = el("div", "attachment");
  const icon = el("span", "file-icon");
  icon.innerHTML = '<svg viewBox="0 0 24 24" width="28" height="28"><path fill="currentColor" d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zm4 18H6V4h7v5h5z"/></svg>';
  box.appendChild(icon);
  const meta = el("div", "file-meta");
  meta.appendChild(el("div", "file-name", isVoiceMessage(att) ? voiceMessageText(null) : att.filename));
  meta.appendChild(el("div", "file-size", formatSize(att.size_bytes)));
  box.appendChild(meta);
  const btn = el("button", "btn-secondary btn-small", "Download");
  btn.onclick = async () => {
    btn.disabled = true;
    btn.textContent = "Fetching";
    await saveAttachment(att);
    btn.disabled = false;
    btn.textContent = "Download";
  };
  box.appendChild(btn);
  return box;
}

async function saveAttachment(att) {
  try {
    const path = await invoke("download_attachment", {
      blob: att.blob,
      filename: att.filename,
      provider: att.provider ?? null,
      sizeBytes: att.size_bytes ?? null,
      serverId: att.server ?? null,
    });
    toast(path ? `Saved to ${path}` : "Download cancelled");
  } catch (e) {
    toast(String(e));
  }
}

let updateStatus = null;

async function loadUpdateStatus() {
  try {
    onUpdateStatus(await invoke("get_update_status"));
  } catch {
    updateStatus = null;
  }
}

function onUpdateStatus(status) {
  const wasReady = updateStatus && updateStatus.phase === "ready";
  updateStatus = status;
  const ready = status && status.phase === "ready";
  $("settings-btn").classList.toggle("update-ready", !!ready);
  $("settings-btn").dataset.i18nTitle = ready ? "nav.settings.updateReady" : "nav.settings";
  $("settings-btn").title = t($("settings-btn").dataset.i18nTitle);
  if (ready && !wasReady && openSettingsLayerId() !== "user-settings") {
    toast(`Commy ${status.version} is ready. Restart from Settings > Updates.`);
  }
  renderUpdateStatus();
}

function renderUpdateStatus() {
  const s = updateStatus || {};
  $("update-version").textContent = s.current ? `Commy ${s.current}` : "Commy";
  $("update-state").textContent = updateStatusText(s);
  const { label, action, disabled } = updateAction(s);
  const btn = $("update-action");
  btn.textContent = label;
  btn.dataset.action = action;
  btn.disabled = disabled;
  btn.className = `${action === "restart" ? "btn-primary" : "btn-secondary"} btn-small update-action`;
  const toggle = $("auto-update-toggle");
  toggle.checked = state.voice ? state.voice.auto_update !== false : true;
  toggle.disabled = !s.can_install;
  toggle.closest(".settings-card").classList.toggle("readonly", !s.can_install);
}

async function runUpdateAction() {
  const action = $("update-action").dataset.action;
  try {
    if (action === "restart") {
      await invoke("restart_to_update");
    } else if (action === "page") {
      if (updateStatus && updateStatus.page) await invoke("open_url", { url: updateStatus.page });
    } else {
      onUpdateStatus({ ...(updateStatus || {}), phase: "checking" });
      onUpdateStatus(await invoke("check_for_updates"));
    }
  } catch (err) {
    toast(String(err));
    loadUpdateStatus();
  }
}

let connectionTestRunning = false;

async function runConnectionTest() {
  if (connectionTestRunning) return;
  connectionTestRunning = true;
  const button = $("conn-test-run");
  const card = button.closest(".conn-test");
  button.disabled = true;
  button.textContent = "Testing...";
  card.classList.add("running");
  try {
    renderConnectionTest(await invoke("run_connection_check"));
  } catch (err) {
    toast(String(err));
  } finally {
    connectionTestRunning = false;
    button.disabled = false;
    button.textContent = "Run Again";
    card.classList.remove("running");
  }
}

function renderConnectionTest(check) {
  const view = describeConnectionCheck(check, { browser: document.documentElement.classList.contains("web-app") });
  const badge = $("conn-test-badge");
  badge.textContent = view.badge;
  badge.className = `conn-badge ${view.tone}`;
  $("conn-test-summary").textContent = view.summary;
  $("conn-test-direct").textContent = view.direct;
  $("conn-test-relay").textContent = view.relay;
  $("conn-test-detail").textContent = view.detail;
}

async function loadDebugLogStatus() {
  try {
    renderDebugLogStatus(await invoke("debug_log_status"));
  } catch {
    renderDebugLogStatus({ verbose: false, saving: false });
  }
}

function renderDebugLogStatus(status) {
  const toggle = $("verbose-log-toggle");
  toggle.checked = !!status.verbose;
  toggle.disabled = !status.saving;
  toggle.closest(".settings-card").classList.toggle("readonly", !status.saving);
  $("log-folder-open").disabled = !status.saving;
  $("log-bundle-save").disabled = !status.saving;
}

async function setVerboseLogging(enabled) {
  try {
    renderDebugLogStatus(await invoke("set_verbose_logging", { enabled }));
  } catch (err) {
    toast(String(err));
    loadDebugLogStatus();
  }
}

async function saveLogBundle() {
  try {
    const saved = await invoke("save_log_bundle");
    if (saved) toast(`Logs saved to ${saved}`);
  } catch (err) {
    toast(String(err));
  }
}

let overlaySettings = null;
let lastOverlayRoster = "";

async function loadOverlaySettings() {
  try {
    overlaySettings = await invoke("get_overlay_settings");
  } catch {
    overlaySettings = null;
  }
}

function emitToOverlay(event, payload) {
  if (!IS_TAURI || !overlaySettings || !overlaySettings.enabled) return;
  window.__TAURI__.event.emitTo("overlay", event, payload).catch(() => {});
}

function overlayRoster() {
  const server = state.currentServer;
  if (!state.voiceChannelId || !state.voiceServer || !server || server.id !== state.voiceServer.id) return null;
  const channel = server.channels.find(c => c.id === state.voiceChannelId);
  if (!channel) return null;
  const ids = channel.voice_user_ids || [];
  return {
    server: server.name,
    channel: channel.name,
    members: channel.voice_users.map((name, i) => {
      const id = ids[i];
      const member = currentServerMember(id);
      const raw = avatarFor(id)?.avatar || "";
      const cached = raw.startsWith("blob:") ? avatarImages.get(raw.slice(5)) : null;
      return {
        id,
        name,
        color: member && member.color,
        emoji: raw.startsWith("emoji:") ? raw.slice(6) : null,
        image: typeof cached === "string" ? cached : null,
      };
    }),
  };
}

function pushOverlayRoster(force = false) {
  const roster = overlayRoster() || (state.voiceChannelId ? null : { members: [] });
  if (!roster) return;
  const key = JSON.stringify(roster);
  if (!force && key === lastOverlayRoster) return;
  lastOverlayRoster = key;
  emitToOverlay("overlay-roster", roster);
}

function pushOverlayChat(authorId, author, content, hasAttachments) {
  if (!overlaySettings || !overlaySettings.show_chat) return;
  const member = currentServerMember(authorId);
  const text = plainText(content) || (hasAttachments ? "sent an attachment" : "");
  if (text) emitToOverlay("overlay-chat", { author, color: member && member.color, text: text.slice(0, 200) });
}

function renderOverlaySettings() {
  const s = overlaySettings;
  if (!s) return;
  $("overlay-enabled").checked = s.enabled;
  $("overlay-options").classList.toggle("disabled", !s.enabled);
  for (const opt of document.querySelectorAll("#overlay-corner .corner-opt")) {
    const on = opt.dataset.corner === s.corner;
    opt.classList.toggle("selected", on);
    opt.setAttribute("aria-checked", String(on));
  }
  $("overlay-everyone").checked = Boolean(s.show_everyone || s.pictures_only);
  $("overlay-everyone").disabled = Boolean(s.pictures_only);
  $("overlay-pictures").checked = Boolean(s.pictures_only);
  $("overlay-opacity").value = Math.round(s.opacity * 100);
  $("overlay-opacity-label").textContent = `${$("overlay-opacity").value}%`;
  syncRangeFill($("overlay-opacity"));
  $("overlay-chat").checked = s.show_chat;
  $("overlay-toggle-key").value = s.toggle_hotkey;
  $("overlay-chat-key").value = s.chat_hotkey;
  invoke("overlay_hotkey_problems").then(taken => {
    const warn = $("overlay-hotkey-warning");
    warn.textContent = taken.length ? `${taken.join(" and ")} ${taken.length === 1 ? "is" : "are"} already used by another app. Pick a different combination.` : "";
    warn.classList.toggle("hidden", !taken.length);
  }).catch(() => {});
}

async function saveOverlaySettings(changes) {
  try {
    overlaySettings = await invoke("set_overlay_settings", { settings: { ...overlaySettings, ...changes } });
    renderOverlaySettings();
    if (overlaySettings.enabled) pushOverlayRoster(true);
  } catch (e) {
    toast(String(e));
    renderOverlaySettings();
  }
}

const MIC_METER_INTERVAL_MS = 80;
let micMeterTimer = null;

function renderVoiceInputSettings() {
  const v = voiceInputState(state.voice);
  const card = $("input-mode").closest(".settings-card");
  card.dataset.mode = v.mode;
  for (const button of $("input-mode").querySelectorAll("button")) {
    const on = button.dataset.mode === v.mode;
    button.classList.toggle("active", on);
    button.setAttribute("aria-checked", String(on));
  }
  $("input-mode-note").textContent = inputModeCopy(v.mode).text;
  $("talk-key").value = v.talkKey;
  const release = $("talk-release");
  if (!release.options.length) {
    for (const [ms, label] of TALK_RELEASE_OPTIONS) release.appendChild(new Option(label, String(ms)));
  }
  release.value = String(v.releaseMs);
  $("sensitivity-auto").checked = v.auto;
  $("sensitivity-label").textContent = sensitivityLabel(v.auto, v.thresholdDb);
  $("sensitivity-block").classList.toggle("auto", v.auto);
  $("mic-meter").classList.toggle("auto", v.auto);
  $("sensitivity-db").value = String(Math.round(v.thresholdDb));
  syncRangeFill($("sensitivity-db"));
  $("mic-meter-mark").style.left = `${meterPercent(v.thresholdDb)}%`;
  $("mute-key").value = v.muteKey;
  $("deafen-key").value = v.deafenKey;
  renderUserPanel();
  invoke("voice_hotkey_problems").then(taken => {
    const warn = $("voice-hotkey-warning");
    warn.textContent = taken.length ? `${taken.join(" and ")} ${taken.length === 1 ? "is" : "are"} already used by another app. Pick a different combination.` : "";
    warn.classList.toggle("hidden", !taken.length);
  }).catch(() => {});
}

async function saveVoiceInput(command, args) {
  try {
    state.voice = await invoke(command, args);
  } catch (e) {
    toast(String(e));
  }
  renderVoiceInputSettings();
}

function wireVoiceInputSettings() {
  for (const button of $("input-mode").querySelectorAll("button")) {
    button.onclick = () => {
      const v = voiceInputState(state.voice);
      saveVoiceInput("set_input_mode", { mode: button.dataset.mode, talkKey: v.talkKey, releaseMs: v.releaseMs });
    };
  }
  wireHotkeyInput($("talk-key"), combo => {
    const v = voiceInputState(state.voice);
    saveVoiceInput("set_input_mode", { mode: v.mode, talkKey: combo, releaseMs: v.releaseMs });
  });
  $("talk-release").addEventListener("change", e => {
    const v = voiceInputState(state.voice);
    saveVoiceInput("set_input_mode", { mode: v.mode, talkKey: v.talkKey, releaseMs: Number(e.target.value) });
  });
  $("sensitivity-auto").addEventListener("change", e => {
    saveVoiceInput("set_sensitivity", { auto: e.target.checked, thresholdDb: voiceInputState(state.voice).thresholdDb });
  });
  $("sensitivity-db").addEventListener("input", e => {
    const db = Number(e.target.value);
    $("sensitivity-label").textContent = sensitivityLabel(false, db);
    $("mic-meter-mark").style.left = `${meterPercent(db)}%`;
  });
  $("sensitivity-db").addEventListener("change", e => {
    saveVoiceInput("set_sensitivity", { auto: false, thresholdDb: Number(e.target.value) });
  });
  wireHotkeyInput($("mute-key"), combo => saveVoiceInput("set_voice_hotkeys", { mute: combo, deafen: voiceInputState(state.voice).deafenKey }), { clearable: true });
  wireHotkeyInput($("deafen-key"), combo => saveVoiceInput("set_voice_hotkeys", { mute: voiceInputState(state.voice).muteKey, deafen: combo }), { clearable: true });
}

function startMicMeter() {
  stopMicMeter();
  micMeterTimer = setInterval(async () => {
    let db = SILENCE_DB;
    try {
      db = await invoke("mic_level");
    } catch {}
    const listening = state.micTesting || Boolean(state.voiceChannelId);
    const v = voiceInputState(state.voice);
    $("mic-meter-fill").style.width = `${listening ? meterPercent(db) : 0}%`;
    $("mic-meter").dataset.state = meterState(db, v.auto, v.thresholdDb, listening);
    $("mic-meter-note").classList.toggle("hidden", listening);
  }, MIC_METER_INTERVAL_MS);
}

function stopMicMeter() {
  clearInterval(micMeterTimer);
  micMeterTimer = null;
}

function setTalking(talking) {
  state.talking = talking;
  $("mute-btn").classList.toggle("talking", talking && !state.voice.muted);
  renderUserPanel();
}

function hotkeyFromEvent(e) {
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return null;
  const parts = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Super");
  if (!parts.length) return null;
  let key = e.code;
  if (/^Key[A-Z]$/.test(key)) key = key.slice(3);
  else if (/^Digit\d$/.test(key)) key = key.slice(5);
  parts.push(key);
  return parts.join("+");
}

function wireHotkeyInput(input, onPick, { clearable = false } = {}) {
  input.addEventListener("focus", () => {
    input.dataset.prev = input.value;
    input.value = "Press keys...";
  });
  input.addEventListener("blur", () => {
    if (input.value === "Press keys...") input.value = input.dataset.prev || "";
  });
  input.addEventListener("keydown", e => {
    e.preventDefault();
    if (e.key === "Escape") {
      input.blur();
      return;
    }
    if (clearable && (e.key === "Backspace" || e.key === "Delete")) {
      input.value = "";
      input.dataset.prev = "";
      input.blur();
      onPick("");
      return;
    }
    const combo = hotkeyFromEvent(e);
    if (!combo) return;
    input.value = combo;
    input.dataset.prev = combo;
    input.blur();
    onPick(combo);
  });
}

let connTimer = null;
let connLast = null;
let connPick = null;

function quality(rtt, loss) {
  if (rtt > 250 || loss > 5) return "bad";
  if (rtt > 120 || loss > 1) return "ok";
  return "good";
}

function formatRate(bytesPerSec) {
  const kbps = (bytesPerSec * 8) / 1000;
  return kbps >= 1000 ? `${(kbps / 1000).toFixed(1)} Mbps` : `${Math.round(kbps)} kbps`;
}

async function refreshConnPop() {
  let stats;
  try {
    stats = await invoke("voice_stats");
  } catch (e) {
    closeConnPop();
    return;
  }
  const now = performance.now();
  const tx = stats.links.reduce((a, l) => a + l.tx_bytes, 0);
  const rx = stats.links.reduce((a, l) => a + l.rx_bytes, 0);
  if (connLast && now > connLast.at) {
    const secs = (now - connLast.at) / 1000;
    $("conn-rates").textContent = `Up ${formatRate(Math.max(0, tx - connLast.tx) / secs)} - Down ${formatRate(Math.max(0, rx - connLast.rx) / secs)}`;
  }
  connLast = { at: now, tx, rx };
  const list = $("conn-links");
  list.replaceChildren();
  const current = pickConnLink(stats.links, connPick);
  connPick = current ? current.user : null;
  const tabs = $("conn-tabs");
  tabs.replaceChildren();
  tabs.classList.toggle("hidden", stats.links.length < 2);
  for (const link of stats.links) {
    const tab = el("button", "conn-tab");
    tab.type = "button";
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", String(link === current));
    tab.title = link.name;
    tab.dataset.quality = quality(link.rtt_ms, link.loss_pct);
    tab.classList.toggle("active", link === current);
    tab.appendChild(makeAvatar(link.user, link.name, "avatar avatar-xs"));
    tab.onclick = () => {
      connPick = link.user;
      refreshConnPop();
    };
    tabs.appendChild(tab);
  }
  for (const link of current ? [current] : []) {
    const row = el("div", "conn-link");
    const head = el("div", "conn-link-head");
    head.appendChild(makeAvatar(link.user, link.name, "avatar avatar-xs"));
    head.appendChild(el("span", "conn-name", link.name));
    const q = quality(link.rtt_ms, link.loss_pct);
    head.appendChild(el("span", `conn-badge ${q}`, { good: "Good", ok: "Fair", bad: "Poor" }[q]));
    row.appendChild(head);
    const grid = el("div", "conn-grid");
    const stat = (label, value) => {
      grid.appendChild(el("span", "conn-label", label));
      grid.appendChild(el("span", "conn-value", value));
    };
    stat("Ping", `${Math.round(link.rtt_ms)} ms`);
    stat("Packet loss", `${link.loss_pct.toFixed(link.loss_pct < 1 ? 2 : 1)}%`);
    stat("Jitter", `${link.jitter_ms.toFixed(1)} ms`);
    const path = link.path === "direct" ? "Connected directly"
      : link.path === "relay" ? "Connected through a relay"
      : link.path === "mixed" ? "Connected directly and through a relay"
      : "Connecting";
    stat("Connection", path);
    row.appendChild(grid);
    list.appendChild(row);
  }
  if (!stats.links.length) list.appendChild(el("div", "conn-empty", "Nobody else is connected yet."));
  $("conn-foot").textContent = `Audio: ${stats.codec}, ${stats.bitrate_kbps} kbps per person`;
  paintSignal(stats);
}

function worstQuality(links) {
  return links.reduce((w, l) => {
    const q = quality(l.rtt_ms, l.loss_pct);
    return q === "bad" || (q === "ok" && w === "good") ? q : w;
  }, "good");
}

function paintSignal(stats) {
  $("vp-signal").dataset.quality = stats ? worstQuality(stats.links) : "good";
}

let signalTimer = null;

async function pollSignal() {
  if (!state.voiceChannelId) return;
  try {
    paintSignal(await invoke("voice_stats"));
  } catch (e) {
    paintSignal(null);
  }
}

function syncSignalPolling(active) {
  if (active && !signalTimer) {
    pollSignal();
    signalTimer = setInterval(pollSignal, 4000);
  } else if (!active && signalTimer) {
    clearInterval(signalTimer);
    signalTimer = null;
    paintSignal(null);
  }
}

function openConnPop() {
  const pop = $("conn-pop");
  closeFloating();
  connLast = null;
  $("conn-rates").textContent = "";
  pop.classList.remove("hidden");
  placePopup(pop, $("vp-conn-meta"), "above");
  refreshConnPop().then(() => placePopup(pop, $("vp-conn-meta"), "above"));
  clearInterval(connTimer);
  connTimer = setInterval(refreshConnPop, 1500);
}

function closeConnPop() {
  $("conn-pop").classList.add("hidden");
  clearInterval(connTimer);
  connTimer = null;
}

function animateToggle(button, off) {
  button.classList.remove("toggle-anim", "slash-in");
  void button.offsetWidth;
  button.classList.add("toggle-anim");
  if (off) button.classList.add("slash-in");
  setTimeout(() => button.classList.remove("toggle-anim", "slash-in"), 450);
}

async function toggleAudioPop(which, anchor) {
  const pop = $("audio-pop");
  if (!pop.classList.contains("hidden") && pop.dataset.which === which) {
    pop.classList.add("hidden");
    return;
  }
  closeFloating();
  pop.dataset.which = which;
  const input = which === "input";
  $("ap-title").textContent = input ? "Input Device" : "Output Device";
  $("ap-volume-title").firstChild.textContent = input ? "Input Volume " : "Output Volume ";
  const select = $("ap-device");
  select.replaceChildren(el("option", null, "Loading devices..."));
  const level = Math.round(((input ? state.voice.input_gain : state.voice.output_volume) ?? 1) * 100);
  $("ap-volume").value = String(level);
  $("ap-volume-label").textContent = `${level}%`;
  syncRangeFill($("ap-volume"));
  $("ap-nc-row").classList.toggle("hidden", !input);
  $("ap-nc").checked = Boolean(state.voice.noise_cancellation);
  pop.classList.remove("hidden");
  placePopup(pop, anchor, "above");
  try {
    const devices = await invoke("list_audio_devices");
    const names = input ? devices.inputs : devices.outputs;
    const current = (input ? state.voice.input_device : state.voice.output_device) || "";
    select.replaceChildren(new Option("Default", ""), ...names.map(n => new Option(n, n)));
    select.value = names.includes(current) ? current : "";
  } catch (e) {
    select.replaceChildren(el("option", null, "No devices found"));
  }
}

function renderMarkdown(text) {
  const root = el("div", "md");
  renderBlocks(root, parseMessage(text));
  return root;
}

function renderBlocks(parent, blocks) {
  for (const block of blocks) {
    const node = renderBlock(block);
    if (block.gapAfter) node.classList.add("gap-after");
    parent.appendChild(node);
  }
}

function renderBlock(block) {
  if (block.type === "codeblock") return renderCodeBlock(block);
  if (block.type === "heading") return renderInlineInto(el(`h${block.level}`, "md-heading"), block.children);
  if (block.type === "quote") {
    const quote = el("blockquote", "md-quote");
    renderBlocks(quote, block.children);
    return quote;
  }
  if (block.type === "list") {
    const list = el(block.ordered ? "ol" : "ul", "md-list");
    if (block.ordered && block.start !== 1) list.start = block.start;
    for (const item of block.items) list.appendChild(renderInlineInto(el("li"), item));
    return list;
  }
  return renderInlineInto(el("div", "md-para"), block.children);
}

function renderCodeBlock(block) {
  const pre = el("pre", "md-code");
  if (block.lang) pre.appendChild(el("span", "md-code-lang", block.lang));
  const code = el("code");
  for (const token of highlightCode(block.code, block.lang)) {
    code.appendChild(token.kind === "plain" ? document.createTextNode(token.text) : el("span", `tok-${token.kind}`, token.text));
  }
  pre.appendChild(code);
  return pre;
}

function renderInlineInto(parent, nodes) {
  for (const node of nodes) parent.appendChild(renderInlineNode(node));
  return parent;
}

const INLINE_TAGS = { strong: "strong", em: "em", underline: "u", strike: "s" };

function renderInlineNode(node) {
  if (node.type === "text") {
    const span = document.createDocumentFragment();
    appendMessageText(span, node.text);
    return span;
  }
  if (node.type === "br") return el("br");
  if (node.type === "code") return el("code", "md-inline-code", node.text);
  if (node.type === "spoiler") return renderSpoiler(node);
  if (node.type === "link") {
    const a = wireOutsideLink(el("a", "md-link"), node.url, { labelled: true });
    a.title = node.url;
    return renderInlineInto(a, node.children);
  }
  if (node.type === "time") return renderTimestamp(node);
  return renderInlineInto(el(INLINE_TAGS[node.type] || "span"), node.children);
}

function renderSpoiler(node) {
  const span = el("span", "spoiler");
  span.setAttribute("role", "button");
  span.title = "Click to reveal";
  renderInlineInto(span, node.children);
  span.addEventListener("click", e => {
    if (span.classList.contains("revealed")) return;
    e.stopPropagation();
    span.classList.add("revealed");
    span.title = "";
  });
  return span;
}

const TIMESTAMP_FORMATS = {
  t: { hour: "2-digit", minute: "2-digit" },
  T: { hour: "2-digit", minute: "2-digit", second: "2-digit" },
  d: { year: "numeric", month: "2-digit", day: "2-digit" },
  D: { year: "numeric", month: "long", day: "numeric" },
  f: { year: "numeric", month: "long", day: "numeric", hour: "2-digit", minute: "2-digit" },
  F: { weekday: "long", year: "numeric", month: "long", day: "numeric", hour: "2-digit", minute: "2-digit" },
};

function relativeTimestamp(date) {
  const seconds = Math.round((date.getTime() - Date.now()) / 1000);
  const units = [["year", 31536000], ["month", 2592000], ["week", 604800], ["day", 86400], ["hour", 3600], ["minute", 60]];
  const rtf = new Intl.RelativeTimeFormat([], { numeric: "auto" });
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return rtf.format(Math.round(seconds / size), unit);
  }
  return rtf.format(seconds, "second");
}

function renderTimestamp(node) {
  const date = new Date(node.unix * 1000);
  const time = el("time", "md-time");
  time.dateTime = date.toISOString();
  time.textContent = node.style === "R" ? relativeTimestamp(date) : date.toLocaleString([], TIMESTAMP_FORMATS[node.style] || TIMESTAMP_FORMATS.f);
  time.title = date.toLocaleString([], TIMESTAMP_FORMATS.F);
  return time;
}

function appendMessageText(parent, text) {
  const urlRe = /https?:\/\/[^\s<>"']+/g;
  let last = 0;
  let match;
  while ((match = urlRe.exec(text))) {
    if (match.index > last) appendRichText(parent, text.slice(last, match.index));
    const url = match[0];
    const a = wireOutsideLink(el("a", null, url), url);
    a.title = `Open ${url}`;
    parent.appendChild(a);
    last = match.index + url.length;
  }
  if (last < text.length) appendRichText(parent, text.slice(last));
}

function mediaProxyOn() {
  return Boolean(account.server_media && window.__TAURI__);
}

function viaAccountServer(url) {
  if (!mediaProxyOn()) return url;
  try {
    if (!isMediaHost(new URL(url).hostname)) return url;
  } catch {
    return url;
  }
  return `${window.__TAURI__.core.convertFileSrc("", "commy-media")}?u=${encodeURIComponent(url)}`;
}

let integrations = { igdb_client_id: "", igdb_configured: false };

async function loadIntegrations() {
  try {
    integrations = await invoke("get_integrations");
  } catch {}
}

function igdbCover(imageId, size = "t_cover_small") {
  return /^[A-Za-z0-9]{1,32}$/.test(imageId || "") ? viaAccountServer(`https://images.igdb.com/igdb/image/upload/${size}/${imageId}.jpg`) : null;
}

const knownGames = new Map();
const pendingGames = new Map();
const missedGames = new Map();
const GAME_RETRY_MS = 10 * 60e3;

function gameKey(name) {
  return String(name || "").toLowerCase();
}

function lookUpGame(name) {
  const key = gameKey(name);
  if (!key) return Promise.resolve(null);
  if (knownGames.has(key)) return Promise.resolve(knownGames.get(key));
  if (Date.now() - (missedGames.get(key) || 0) < GAME_RETRY_MS) return Promise.resolve(null);
  if (!pendingGames.has(key)) {
    pendingGames.set(key, invoke("game_details", { name })
      .catch(() => null)
      .then(hit => {
        if (hit) knownGames.set(key, hit);
        else missedGames.set(key, Date.now());
        return hit || null;
      })
      .finally(() => pendingGames.delete(key)));
  }
  return pendingGames.get(key);
}

function steamButton(appId) {
  const url = steamStoreUrl(appId);
  if (!url) return null;
  const btn = el("button", "ac-steam");
  btn.type = "button";
  btn.append(svgIcon("steam", 18), el("span", null, "Get on Steam"));
  btn.onclick = e => {
    e.stopPropagation();
    invoke("open_url", { url }).catch(err => toast(String(err)));
  };
  return btn;
}

function coverSlot(className, imageId, size) {
  const slot = el("div", className);
  const src = igdbCover(imageId, size);
  if (!src) {
    slot.appendChild(svgIcon("gamepad", 22));
    return slot;
  }
  const img = el("img");
  img.alt = "";
  img.onerror = () => img.replaceWith(svgIcon("gamepad", 22));
  img.src = src;
  slot.appendChild(img);
  return slot;
}

function activityTime(person) {
  const time = el("div", "ac-time");
  time.appendChild(svgIcon("gamepad", 14));
  if (person.activity_since) time.appendChild(liveElapsed(person.activity_since, "clock"));
  else time.appendChild(el("span", null, "In game"));
  return time;
}

function activityCard(person) {
  const game = gameFromActivity(person.activity);
  const details = knownGames.get(gameKey(game));
  const art = activityArt(person, details);
  const card = el("div", "play-card");
  const glow = el("div", "ac-glow");
  const glowSrc = igdbCover(art, "t_cover_big");
  if (glowSrc) glow.style.backgroundImage = `url("${glowSrc}")`;
  const main = el("div", "ac-main");
  const info = el("div", "ac-info");
  info.append(el("div", "ac-game", game), activityTime(person));
  main.append(coverSlot("ac-cover", art, "t_cover_big"), info);
  card.append(glow, main);
  const steam = details && steamButton(details.steam);
  if (steam) card.appendChild(steam);
  return card;
}

function fillActivity(box, person, afterLookup) {
  box.replaceChildren();
  const game = isShowingActivity(person) && gameFromActivity(person.activity);
  box.classList.toggle("hidden", !game);
  if (!game) return;
  box.append(el("div", "ac-label", "Playing"), activityCard(person));
  if (knownGames.has(gameKey(game))) return;
  lookUpGame(game).then(() => {
    if (!knownGames.has(gameKey(game)) || !box.isConnected || box.dataset.user !== person.id) return;
    fillActivity(box, person, afterLookup);
    if (afterLookup) afterLookup();
  });
}

function showActivityIn(box, person, afterLookup) {
  box.dataset.user = person ? person.id : "";
  fillActivity(box, person || {}, afterLookup);
}

function keepOnScreen(node) {
  const r = node.getBoundingClientRect();
  if (r.bottom > window.innerHeight - 8) node.style.top = `${Math.max(8, window.innerHeight - r.height - 8)}px`;
}

function statusBesideGame(person) {
  const own = [person.status_emoji, person.custom_status].filter(Boolean).join(" ");
  return [backLine(person), own].filter(Boolean).join(" · ");
}

function activityChip(member) {
  const game = gameFromActivity(member.activity);
  const details = knownGames.get(gameKey(game));
  const chip = el("button", "now-card");
  chip.type = "button";
  chip.title = `${member.name} is playing ${game}`;
  const text = el("div", "acc-text");
  const who = el("div", "acc-who");
  const name = el("span", null, member.name);
  paintRole(name, member.color);
  who.append(makeAvatar(member.id, member.name, "avatar acc-avatar"), name);
  text.append(who, el("div", "ac-game", game), activityTime(member));
  const cover = coverSlot("acc-cover", activityArt(member, details), "t_cover_small");
  chip.append(text, cover);
  chip.onclick = e => {
    e.stopPropagation();
    openMemberCard(member, chip);
  };
  if (!state.user || member.id !== state.user.id) {
    chip.oncontextmenu = e => {
      e.preventDefault();
      openMemberMenu(member, e.clientX, e.clientY);
    };
  }
  if (!details) {
    lookUpGame(game).then(found => {
      if (member.activity_cover) return;
      if (found && found.cover && chip.isConnected) cover.replaceWith(coverSlot("acc-cover", found.cover, "t_cover_small"));
    });
  }
  return chip;
}

function renderPlayingGroup(list, playing) {
  if (!playing.length) return;
  list.appendChild(el("div", "member-group", `Playing - ${playing.length}`));
  const group = el("div", "playing-now");
  for (const member of playing) group.appendChild(activityChip(member));
  list.appendChild(group);
}

function renderIntegrations() {
  $("igdb-client-id").value = integrations.igdb_client_id || "";
  $("igdb-client-secret").value = "";
  $("igdb-client-secret").placeholder = integrations.igdb_configured ? "Saved (leave blank to keep)" : "";
  $("igdb-state").textContent = integrations.igdb_configured ? "Connected" : "";
  $("igdb-state").classList.toggle("on", integrations.igdb_configured);
  $("igdb-remove").classList.toggle("hidden", !integrations.igdb_configured);
  $("igdb-result").textContent = "";
}

async function saveIgdb(remove) {
  const result = $("igdb-result");
  result.className = "integration-result";
  result.textContent = remove ? "" : "Checking with Twitch and IGDB...";
  try {
    const sample = await invoke("set_igdb_credentials", {
      clientId: remove ? "" : $("igdb-client-id").value,
      clientSecret: remove ? null : $("igdb-client-secret").value || null,
    });
    await loadIntegrations();
    renderIntegrations();
    if (!remove) {
      result.textContent = sample.length ? `Working: found ${sample[0].name}` : "Connected";
      result.classList.add("ok");
    }
  } catch (e) {
    result.textContent = String(e);
    result.classList.add("bad");
  }
}

let streamDeck = { enabled: false, status: "off", decks: [] };
let streamDeckPairing = null;

const STREAM_DECK_STATE_LABELS = { listening: "On", starting: "Starting", busy: "Unavailable" };

async function loadStreamDeck() {
  try {
    streamDeck = await invoke("get_stream_deck");
  } catch {}
}

function renderStreamDeck() {
  $("stream-deck-enabled").checked = streamDeck.enabled;
  const label = streamDeck.enabled ? STREAM_DECK_STATE_LABELS[streamDeck.status] || "" : "";
  $("stream-deck-state").textContent = label;
  $("stream-deck-state").classList.toggle("on", streamDeck.enabled && streamDeck.status === "listening");
  $("stream-deck-note").textContent = streamDeck.enabled && streamDeck.status === "busy"
    ? "Another Commy app on this computer is already connected to Stream Deck. Close it, then turn this off and on again."
    : "";
  $("stream-deck-decks").replaceChildren(...streamDeck.decks.map(streamDeckRow));
}

function streamDeckRow(deck) {
  const row = el("div", "stream-deck-deck");
  row.appendChild(el("span", "stream-deck-deck-name", deck.name));
  row.appendChild(el("span", "stream-deck-deck-since", `Allowed ${new Date(deck.paired_at).toLocaleDateString()}`));
  const remove = el("button", "btn-link danger", "Remove");
  remove.onclick = async () => {
    try {
      streamDeck = await invoke("forget_stream_deck", { id: deck.id });
      renderStreamDeck();
    } catch (e) {
      toast(String(e));
    }
  };
  row.appendChild(remove);
  return row;
}

async function setStreamDeckEnabled(enabled) {
  try {
    streamDeck = await invoke("set_stream_deck_enabled", { enabled });
  } catch (e) {
    toast(String(e));
  }
  renderStreamDeck();
}

function onStreamDeckPair(prompt) {
  streamDeckPairing = prompt.request;
  $("stream-deck-pair-body").textContent =
    `${prompt.name} wants to control Commy: play and add sounds, mute, switch channels and more. Only allow it if you just set up the Commy plugin on this computer.`;
  showModal("modal-stream-deck");
}

async function onStreamDeckCommand(command) {
  try {
    if (command.action === "toggle_camera") {
      if (state.voiceChannelId) await (state.cameraOn ? stopCamera() : startCamera());
    } else if (command.action === "toggle_stream") {
      await toggleShare();
    } else if (command.action === "open_channel") {
      await openNotificationTarget(`channel:${command.server}:${command.channel}`);
    } else if (command.action === "join_voice") {
      if (!state.currentServer || state.currentServer.id !== command.server) await selectServer(command.server);
      const channel = state.currentServer && state.currentServer.channels.find(c => c.id === command.channel);
      if (channel) await toggleVoice(channel);
    }
  } catch (e) {
    toast(String(e));
  }
}

function onStreamDeckPairClosed(request) {
  if (streamDeckPairing !== request) return;
  streamDeckPairing = null;
  if (!$("modal-stream-deck").classList.contains("hidden")) hideModals();
}

function answerStreamDeckPairing(allow) {
  const request = streamDeckPairing;
  if (request === null) return;
  streamDeckPairing = null;
  invoke("answer_stream_deck_pairing", { request, allow }).catch(e => toast(String(e)));
}

let gameSearchTimer = null;
let gameSearchSeq = 0;

function closeGameSearch() {
  $("game-search").classList.add("hidden");
  $("game-search").replaceChildren();
}

function queueGameSearch(query) {
  clearTimeout(gameSearchTimer);
  if (query.trim().length < 2) {
    closeGameSearch();
    return;
  }
  gameSearchTimer = setTimeout(async () => {
    const seq = ++gameSearchSeq;
    let hits = [];
    try {
      hits = await invoke("search_games", { query });
    } catch (e) {
      hits = [];
    }
    if (seq !== gameSearchSeq) return;
    const box = $("game-search");
    box.replaceChildren();
    for (const hit of hits.slice(0, 6)) {
      const row = el("button", "game-hit");
      const cover = igdbCover(hit.cover, "t_thumb");
      const img = el(cover ? "img" : "span", "game-hit-cover");
      if (cover) img.src = cover;
      row.appendChild(img);
      row.appendChild(el("span", "game-hit-name", hit.name));
      if (hit.year) row.appendChild(el("span", "game-hit-year", String(hit.year)));
      row.onmousedown = e => {
        e.preventDefault();
        $("game-tag-input").value = "";
        closeGameSearch();
        addGameTag(hit.name, { name: hit.name, igdb_id: hit.id, cover: hit.cover || null });
      };
      box.appendChild(row);
    }
    box.classList.toggle("hidden", !hits.length);
  }, 300);
}

let gameTagDraft = [];
let gameTagMetaDraft = [];

function addGameTag(name, meta) {
  if (gameTagDraft.some(t => t.toLowerCase() === name.toLowerCase())) return;
  const metas = gameTagMetaDraft.filter(m => m.name !== name);
  if (meta) metas.push(meta);
  saveGameTags([...gameTagDraft, name], metas);
}

function tagCover(server, name, size) {
  const meta = (server.game_tag_meta || []).find(m => m.name === name);
  return meta ? igdbCover(meta.cover, size) : null;
}

function gameChip(server, tag, cls) {
  const chip = el("span", cls, tag);
  const cover = server && tagCover(server, tag, "t_thumb");
  if (cover) {
    const img = el("img", "game-chip-cover");
    img.src = cover;
    chip.prepend(img);
  }
  return chip;
}

function renderGameTagEditor(server) {
  gameTagDraft = [...(server.game_tags || [])];
  gameTagMetaDraft = [...(server.game_tag_meta || [])];
  const editable = Boolean(server.can_manage_server);
  $("game-tag-input").classList.toggle("hidden", !editable || gameTagDraft.length >= 5);
  paintGameTagChips(editable);
}

function paintGameTagChips(editable) {
  const chips = $("game-tag-chips");
  chips.replaceChildren();
  for (const tag of gameTagDraft) {
    const chip = gameChip(state.currentServer, tag, "game-chip");
    if (editable) {
      const x = el("button", "game-chip-x");
      x.title = `Remove ${tag}`;
      x.appendChild(svgIcon("close", 12));
      x.onclick = () => saveGameTags(gameTagDraft.filter(t => t !== tag), gameTagMetaDraft.filter(m => m.name !== tag));
      chip.appendChild(x);
    }
    chips.appendChild(chip);
  }
  if (!gameTagDraft.length && !editable) chips.appendChild(el("span", "setting-sub", "No games tagged"));
}

async function saveGameTags(tags, meta = gameTagMetaDraft) {
  if (!state.currentServer) return;
  try {
    await invoke("set_game_tags", { serverId: state.currentServer.id, tags, meta: meta.filter(m => tags.includes(m.name)) });
    await refreshCurrentServer();
    renderGameTagEditor(state.currentServer);
  } catch (e) {
    toast(String(e));
  }
}

function upcomingEventCount(server) {
  const now = Date.now();
  return (server.events || []).filter(e => new Date(e.end || e.start).getTime() >= now).length;
}

function renderServerExtras(list, server) {
  if (server.game_tags && server.game_tags.length) {
    const row = el("div", "sidebar-game-tags");
    for (const tag of server.game_tags) {
      row.appendChild(gameChip(server, tag, "game-chip small"));
    }
    list.appendChild(row);
  }
  const upcoming = upcomingEventCount(server);
  const events = el("div", "channel sidebar-link");
  events.appendChild(el("span", "channel-icon")).appendChild(svgIcon("calendar", 18));
  events.appendChild(el("span", "channel-label", upcoming ? `${upcoming} Event${upcoming === 1 ? "" : "s"}` : "Events"));
  events.onclick = () => openEvents();
  list.appendChild(events);
  if (server.rules) {
    const rules = el("div", "channel sidebar-link");
    rules.appendChild(el("span", "channel-icon")).appendChild(svgIcon("book", 18));
    rules.appendChild(el("span", "channel-label", "Server Rules"));
    if (server.needs_rules) rules.appendChild(el("span", "sidebar-link-badge", "Accept"));
    rules.onclick = () => openRules();
    list.appendChild(rules);
  }
  if (selfAssignableRoles(server).length) {
    const roles = el("div", "channel sidebar-link");
    roles.appendChild(el("span", "channel-icon")).appendChild(svgIcon("roles", 18));
    roles.appendChild(el("span", "channel-label", "Roles"));
    roles.title = "Pick roles for yourself";
    roles.onclick = () => openChooseRoles();
    list.appendChild(roles);
  }
  list.appendChild(el("div", "sidebar-sep"));
}

const rulesPrompted = new Set();

function openRules() {
  const server = state.currentServer;
  if (!server || !server.rules) return;
  $("rules-title").textContent = `${server.name} Rules`;
  $("rules-body").replaceChildren(renderMarkdown(server.rules));
  $("rules-accept").classList.toggle("hidden", !server.needs_rules);
  showModal("modal-rules");
}

async function acceptRules() {
  const server = state.currentServer;
  if (!server) return;
  try {
    await invoke("accept_rules", { serverId: server.id });
    await refreshCurrentServer();
    rolePromptPending.delete(state.currentServer.id);
    if (state.currentServer.onboarding) openOnboarding("join");
    else if (selfAssignableRoles(state.currentServer).length) openServerWelcome("roles");
    else {
      hideModals();
      toast("Thanks! You can chat now");
    }
  } catch (e) {
    toast(String(e));
  }
}

function openServerWelcome(step) {
  const server = state.currentServer;
  if (!server) return;
  closeFloating();
  const hasRoles = selfAssignableRoles(server).length > 0;
  const twoSteps = Boolean(server.rules) && (hasRoles || Boolean(server.onboarding));
  $("sw-steps").lastElementChild.textContent = server.onboarding ? "Questions" : "Roles";
  paintServerIcon($("sw-icon"), server);
  $("sw-title").textContent = step === "rules" ? `Welcome to ${server.name}` : `You are in, ${state.user ? state.user.username : "friend"}`;
  $("sw-sub").textContent = step === "rules"
    ? "This server asks new members to read its rules before posting."
    : `${server.name} has roles you can give yourself.`;
  $("sw-steps").classList.toggle("hidden", !twoSteps);
  for (const dot of $("sw-steps").children) {
    dot.classList.toggle("active", dot.dataset.step === step);
    dot.classList.toggle("done", step === "roles" && dot.dataset.step === "rules");
  }
  $("sw-rules").classList.toggle("hidden", step !== "rules");
  $("sw-roles").classList.toggle("hidden", step !== "roles");
  if (step === "rules") $("sw-rules").replaceChildren(renderMarkdown(server.rules || ""));
  else renderRolePicker(server, $("sw-role-list"));
  $("sw-later").classList.toggle("hidden", step === "rules" && server.needs_rules);
  $("sw-next").textContent = step === "rules" ? "I Have Read and Agree" : "Done";
  $("sw-next").onclick = step === "rules" ? acceptRules : hideModals;
  showModal("modal-server-welcome");
  $("sw-next").focus();
}

const rolePromptPending = new Set();

async function enterJoinedServer(summary) {
  rolePromptPending.add(summary.id);
  await refreshServers();
  await selectServer(summary.id);
  toast("Joined server, syncing from peers");
}

function maybePromptRoles() {
  const server = state.currentServer;
  if (!server || !rolePromptPending.has(server.id) || server.needs_rules) return;
  if (!server.onboarding && !selfAssignableRoles(server).length) return;
  if (!$("modal-backdrop").classList.contains("hidden")) return;
  rolePromptPending.delete(server.id);
  if (server.onboarding) openOnboarding("join");
  else openServerWelcome("roles");
}

function syncRulesGate() {
  const gated = state.view === "server" && Boolean(state.currentServer && state.currentServer.needs_rules);
  $("rules-gate").classList.toggle("hidden", !gated);
  const where = { view: state.view, server: state.currentServer, channel: state.currentChannel };
  const timedOut = gated ? null : timeoutLock(where);
  const accessLock = gated || timedOut ? null : channelAccessLock(where);
  const lock = gated ? null : timedOut || accessLock || composerLock(where);
  const aged = state.view === "server" && ageGated(state.currentChannel);
  $("announce-gate").classList.toggle("hidden", !lock || aged);
  if (lock) {
    $("announce-gate-icon").replaceChildren(svgIcon(timedOut ? "clock" : accessLock ? "lock" : "megaphone", 22));
    $("announce-gate-title").textContent = lock.title;
    const text = $("announce-gate-text");
    text.textContent = lock.text;
    if (timedOut) {
      const left = el("strong", "timeout-left", timeoutLeftText(timedOut.until));
      left.dataset.until = String(timedOut.until);
      text.append(left, document.createTextNode("."));
    }
  }
  if (state.view === "server") document.querySelector(".composer").classList.toggle("hidden", gated || aged || Boolean(lock) || isForumChannel(state.currentChannel));
  if (gated && !rulesPrompted.has(state.currentServer.id)) {
    rulesPrompted.add(state.currentServer.id);
    openServerWelcome("rules");
  }
  if (!gated) maybePromptRoles();
}

function awaitedText(count) {
  const n = Number(count);
  if (!Number.isInteger(n) || n <= 0) return null;
  return n === 1
    ? "1 message is waiting for you. It arrives when someone who has it comes online."
    : `${n} messages are waiting for you. They arrive when someone who has them comes online.`;
}

let awaitedSeq = 0;

async function syncAwaitedBar() {
  const seq = ++awaitedSeq;
  const serverId = state.view === "server" && state.currentServer ? state.currentServer.id : null;
  let text = null;
  if (serverId) {
    try {
      text = awaitedText(await invoke("get_awaited", { serverId }));
    } catch {}
  }
  if (seq !== awaitedSeq) return;
  $("awaited-text").textContent = text || "";
  $("awaited-bar").classList.toggle("hidden", !text);
}

const onboardingFlow = { mode: "join", onboarding: null, picks: [], pages: [], page: 0 };

function myHeldRoles(server) {
  const me = state.user && (server.members || []).find(m => m.id === state.user.id);
  return me ? me.roles || [] : [];
}

function roleDots(server, roleIds) {
  const wrap = el("span", "ob-answer-roles");
  const names = [];
  for (const id of roleIds || []) {
    const role = (server.roles || []).find(r => r.id === id);
    if (!role) continue;
    const dot = el("span", "role-dot");
    if (role.color) dot.style.background = `#${role.color}`;
    wrap.appendChild(dot);
    names.push(role.name);
  }
  if (names.length) wrap.title = `Gives you ${names.join(", ")}`;
  return wrap;
}

function openOnboarding(mode, draft) {
  const server = state.currentServer;
  const onboarding = mode === "preview" ? draft : server && server.onboarding;
  if (!server || !onboarding || !onboarding.questions.length) return;
  closeFloating();
  Object.assign(onboardingFlow, {
    mode,
    onboarding,
    picks: inferPicks(onboarding, myHeldRoles(server)),
    pages: onboardingPages(onboarding.questions.length),
    page: 0,
  });
  paintServerIcon($("ob-icon"), server);
  const who = state.user ? state.user.username : "friend";
  const copy = {
    join: [`Welcome in, ${who}`, `${server.name} uses your answers to hand out roles and open the right channels.`],
    change: ["Your answers", "Change what you picked when you joined. Your roles update right away."],
    preview: ["Welcome in, friend", "This is what new members see after the rules. Nothing picked here is saved."],
  }[mode];
  $("ob-title").textContent = copy[0];
  $("ob-sub").textContent = copy[1];
  $("ob-preview-badge").classList.toggle("hidden", mode !== "preview");
  $("ob-later").textContent = { join: "Later", change: "Cancel", preview: "Close" }[mode];
  renderOnboardingPage();
  showModal("modal-onboarding");
  $("ob-next").focus();
}

function renderOnboardingPage() {
  const server = state.currentServer;
  const { onboarding, pages, page, picks } = onboardingFlow;
  const steps = $("ob-steps");
  steps.replaceChildren();
  steps.classList.toggle("hidden", pages.length < 2);
  pages.forEach((_, i) => {
    const dot = el("span", "sw-step", String(i + 1));
    dot.classList.toggle("active", i === page);
    dot.classList.toggle("done", i < page);
    steps.appendChild(dot);
  });
  const list = $("ob-questions");
  list.replaceChildren();
  for (const qi of pages[page] || []) {
    const question = onboarding.questions[qi];
    const block = el("div", "ob-question");
    block.appendChild(el("div", "ob-question-title", question.title));
    if (question.description) block.appendChild(el("div", "ob-question-sub", question.description));
    const answers = el("div", "ob-answers");
    question.answers.forEach((answer, ai) => {
      const row = el("button", "ob-answer" + (question.multiple ? " multiple" : ""));
      row.type = "button";
      row.classList.toggle("picked", (picks[qi] || []).includes(ai));
      row.appendChild(el("span", "ob-check")).appendChild(svgIcon("check", 14));
      row.appendChild(el("span", "ob-answer-label", answer.label));
      row.appendChild(roleDots(server, answer.roles));
      row.onclick = () => {
        picks[qi] = togglePick(question, picks[qi], ai);
        for (const [i, node] of [...answers.children].entries()) node.classList.toggle("picked", picks[qi].includes(i));
      };
      answers.appendChild(row);
    });
    block.appendChild(answers);
    list.appendChild(block);
  }
  list.scrollTop = 0;
  $("ob-back").classList.toggle("hidden", page === 0);
  $("ob-next").textContent = page >= pages.length - 1 ? "Done" : "Next";
}

function turnOnboardingPage(delta) {
  const next = onboardingFlow.page + delta;
  if (next < 0 || next >= onboardingFlow.pages.length) return;
  onboardingFlow.page = next;
  renderOnboardingPage();
}

async function advanceOnboarding() {
  if (onboardingFlow.page < onboardingFlow.pages.length - 1) return turnOnboardingPage(1);
  const server = state.currentServer;
  if (!server) return hideModals();
  if (onboardingFlow.mode === "preview") {
    hideModals();
    toast("Preview only, nothing was saved");
    return;
  }
  const { grant, revoke } = answerDiff(onboardingFlow.onboarding, onboardingFlow.picks, myHeldRoles(server));
  if (!grant.length && !revoke.length) {
    hideModals();
    toast(onboardingFlow.mode === "join" ? "You are all set" : "Your answers are unchanged");
    return;
  }
  try {
    await invoke("answer_onboarding", { serverId: server.id, grant, revoke });
    await refreshCurrentServer();
    hideModals();
    toast(onboardingFlow.mode === "join" ? "You are all set" : "Answers saved");
  } catch (e) {
    toast(String(e));
  }
}

let onboardingDraft = { questions: [] };
let onboardingDraftSource = null;

function renderOnboardingSettings(server) {
  const source = JSON.stringify(server.onboarding || null);
  if (source !== onboardingDraftSource) {
    onboardingDraftSource = source;
    onboardingDraft = server.onboarding ? structuredClone(server.onboarding) : { questions: [] };
  }
  $("ob-remove").classList.toggle("hidden", !server.onboarding);
  paintOnboardingEditor(server);
}

function syncOnboardingCount() {
  const usage = onboardingUsage(onboardingDraft);
  const count = $("ob-count");
  count.textContent = `${usage.bytes} / ${ONBOARDING_LIMITS.textBytes}`;
  count.classList.toggle("over", usage.bytes > ONBOARDING_LIMITS.textBytes);
}

function onboardingTextField(value, placeholder, maxLength, onInput) {
  const input = el("input");
  input.type = "text";
  input.value = value || "";
  input.placeholder = placeholder;
  input.maxLength = maxLength;
  input.oninput = () => {
    onInput(input.value);
    syncOnboardingCount();
  };
  return input;
}

function iconButton(icon, title, onClick) {
  const btn = el("button", "ob-icon-btn");
  btn.type = "button";
  btn.title = title;
  btn.appendChild(svgIcon(icon, 16));
  btn.onclick = onClick;
  return btn;
}

function paintOnboardingAnswer(server, question, answer, index, repaint) {
  const row = el("div", "ob-answer-row");
  row.appendChild(onboardingTextField(answer.label, `Answer ${index + 1}`, ONBOARDING_LIMITS.answerChars, v => { answer.label = v; }));
  row.appendChild(iconButton("close", "Remove answer", () => {
    question.answers.splice(index, 1);
    repaint();
  }));
  const chips = el("div", "ob-role-chips");
  const eligible = onboardingRoles(server.roles);
  answer.roles = (answer.roles || []).filter(id => eligible.some(r => r.id === id));
  for (const id of answer.roles) {
    const role = eligible.find(r => r.id === id);
    const chip = el("span", "ob-role-chip");
    const dot = el("span", "role-dot");
    if (role.color) dot.style.background = `#${role.color}`;
    chip.append(dot, document.createTextNode(role.name));
    const x = el("button", "game-chip-x");
    x.type = "button";
    x.title = `Stop giving ${role.name}`;
    x.appendChild(svgIcon("close", 12));
    x.onclick = () => {
      answer.roles = answer.roles.filter(r => r !== id);
      repaint();
    };
    chip.appendChild(x);
    chips.appendChild(chip);
  }
  const left = eligible.filter(r => !answer.roles.includes(r.id));
  if (left.length && answer.roles.length < ONBOARDING_LIMITS.rolesPerAnswer) {
    const select = el("select", "compact-select");
    select.title = "Give a role to members who pick this";
    fillOptions(select, [{ value: "", label: answer.roles.length ? "Add another role" : "Gives no role" }, ...left.map(r => ({ value: r.id, label: r.name }))], "");
    select.onchange = () => {
      if (!select.value) return;
      answer.roles.push(select.value);
      repaint();
    };
    chips.appendChild(select);
  } else if (!eligible.length) {
    chips.appendChild(el("span", "setting-sub", "Create a role under Roles to hand it out here"));
  }
  row.appendChild(chips);
  return row;
}

function paintOnboardingQuestion(server, question, index, repaint) {
  const card = el("div", "settings-card ob-card");
  const head = el("div", "ob-card-head");
  head.appendChild(el("div", "setting-title", `Question ${index + 1}`));
  const type = el("select", "compact-select");
  fillOptions(type, [{ value: "single", label: "Pick one" }, { value: "multiple", label: "Pick any" }], question.multiple ? "multiple" : "single");
  type.onchange = () => { question.multiple = type.value === "multiple"; };
  head.appendChild(type);
  head.appendChild(iconButton("trash", "Remove question", () => {
    onboardingDraft.questions.splice(index, 1);
    repaint();
  }));
  card.appendChild(head);
  const title = el("div", "ob-field");
  title.appendChild(onboardingTextField(question.title, "What do you want to ask?", ONBOARDING_LIMITS.titleChars, v => { question.title = v; }));
  card.appendChild(title);
  const sub = el("div", "ob-field");
  sub.appendChild(onboardingTextField(question.description, "Optional one-line description", ONBOARDING_LIMITS.descriptionChars, v => { question.description = v; }));
  card.appendChild(sub);
  const rows = el("div", "ob-answer-rows");
  question.answers.forEach((answer, ai) => rows.appendChild(paintOnboardingAnswer(server, question, answer, ai, repaint)));
  card.appendChild(rows);
  if (question.answers.length < ONBOARDING_LIMITS.answers) {
    const add = el("button", "btn-secondary btn-small ob-add-answer", "Add Answer");
    add.type = "button";
    add.onclick = () => {
      question.answers.push({ label: "", roles: [] });
      repaint();
    };
    card.appendChild(add);
  }
  return card;
}

function paintOnboardingEditor(server) {
  const editor = $("ob-editor");
  editor.replaceChildren();
  const repaint = () => paintOnboardingEditor(server);
  onboardingDraft.questions.forEach((q, i) => editor.appendChild(paintOnboardingQuestion(server, q, i, repaint)));
  if (!onboardingDraft.questions.length) {
    editor.appendChild(el("div", "manage-empty", "No questions yet. Add one to ask new members something when they join."));
  }
  $("ob-add-question").classList.toggle("hidden", onboardingDraft.questions.length >= ONBOARDING_LIMITS.questions);
  syncOnboardingCount();
}

function addOnboardingQuestion() {
  if (!state.currentServer || onboardingDraft.questions.length >= ONBOARDING_LIMITS.questions) return;
  onboardingDraft.questions.push({ title: "", description: "", multiple: false, answers: [{ label: "", roles: [] }, { label: "", roles: [] }] });
  paintOnboardingEditor(state.currentServer);
  const inputs = $("ob-editor").querySelectorAll(".ob-card input");
  const first = inputs[inputs.length - 2 - onboardingDraft.questions[onboardingDraft.questions.length - 1].answers.length];
  if (first) first.focus();
}

function tidyOnboardingDraft(server) {
  return tidyOnboarding(onboardingDraft, onboardingRoles(server.roles).map(r => r.id));
}

async function saveOnboarding(remove) {
  const server = state.currentServer;
  if (!server) return;
  const problems = remove ? [] : onboardingProblems(onboardingDraft);
  if (problems.length) return toast(problems[0]);
  try {
    await invoke("set_onboarding", { serverId: server.id, onboarding: remove ? null : tidyOnboardingDraft(server) });
    await refreshCurrentServer();
    onboardingDraftSource = null;
    renderOnboardingSettings(state.currentServer);
    toast(remove ? "Onboarding questions removed" : "Onboarding saved");
  } catch (e) {
    toast(String(e));
  }
}

function previewOnboarding() {
  const server = state.currentServer;
  if (!server) return;
  const problems = onboardingProblems(onboardingDraft);
  if (problems.length) return toast(problems[0]);
  openOnboarding("preview", tidyOnboardingDraft(server));
}

function renderRulesSettings(server) {
  $("rules-input").value = server.rules || "";
  $("rules-remove").classList.toggle("hidden", !server.rules);
  syncRulesPreview();
}

function syncRulesPreview() {
  const text = $("rules-input").value;
  $("rules-preview").replaceChildren(text.trim() ? renderMarkdown(text) : el("div", "setting-sub", "Preview shows here"));
  const bytes = new TextEncoder().encode(text).length;
  $("rules-count").textContent = `${bytes} / 3000`;
  $("rules-count").classList.toggle("over", bytes > 3000);
}

async function saveRules(remove) {
  const server = state.currentServer;
  if (!server) return;
  try {
    await invoke("set_rules", { serverId: server.id, rules: remove ? null : $("rules-input").value });
    await refreshCurrentServer();
    renderRulesSettings(state.currentServer);
    toast(remove ? "Rules removed" : "Rules saved");
  } catch (e) {
    toast(String(e));
  }
}

function renderAutomodSettings(server) {
  const rules = server.automod || {};
  $("automod-status").textContent = automodSummary(server.automod);
  $("automod-terms").value = (rules.blocked_terms || []).join("\n");
  syncAutomodCount();
  $("automod-links").checked = Boolean(rules.block_links);
  fillOptions($("automod-mentions"), mentionOptions(rules.max_mentions || 0), String(rules.max_mentions || 0));
  fillOptions($("automod-burst"), burstOptions(rules.burst), burstKey(rules.burst));
  const exempt = new Set(rules.exempt_roles || []);
  const roles = [...(server.roles || [])].sort((a, b) => b.position - a.position);
  $("automod-roles").replaceChildren(...roles.map(role => {
    const label = el("label");
    const box = el("input");
    box.type = "checkbox";
    box.value = role.id;
    box.checked = exempt.has(role.id);
    label.append(box, el("span", null, role.name));
    return label;
  }));
}

function fillOptions(select, options, value) {
  select.replaceChildren(...options.map(o => {
    const option = el("option", null, o.label);
    option.value = o.value;
    return option;
  }));
  select.value = value;
}

function syncAutomodCount() {
  const usage = termsUsage(parseBlockedTerms($("automod-terms").value));
  $("automod-count").textContent = termsUsageText(usage);
  $("automod-count").classList.toggle("over", usage.over);
}

async function saveAutomod() {
  const server = state.currentServer;
  if (!server) return;
  const exemptRoles = [...$("automod-roles").querySelectorAll("input:checked")].map(box => box.value);
  if (exemptRoles.length > AUTOMOD_MAX_EXEMPT_ROLES) {
    toast(`Pick up to ${AUTOMOD_MAX_EXEMPT_ROLES} exempt roles`);
    return;
  }
  const rules = automodPayload({
    termsText: $("automod-terms").value,
    blockLinks: $("automod-links").checked,
    mentionLimit: $("automod-mentions").value,
    burst: $("automod-burst").value,
    exemptRoles,
  });
  try {
    await invoke("set_automod", { serverId: server.id, rules });
    await refreshCurrentServer();
    renderAutomodSettings(state.currentServer);
    toast("Auto moderation saved");
  } catch (e) {
    toast(String(e));
  }
}

const eventsView = { mode: "upcoming", month: null, day: null };
let editingEventId = null;

function openEvents() {
  if (!state.currentServer) return;
  closeFloating();
  const first = new Date();
  eventsView.month = new Date(first.getFullYear(), first.getMonth(), 1);
  eventsView.day = null;
  $("events-title").textContent = `${state.currentServer.name} Events`;
  $("event-create").classList.toggle("hidden", !canCreateEvents(state.currentServer));
  $("events-layer").classList.remove("hidden");
  renderEvents();
}

function closeEvents() {
  $("events-layer").classList.add("hidden");
}

function renderEvents() {
  const server = state.currentServer;
  if (!server || $("events-layer").classList.contains("hidden")) return;
  for (const tab of document.querySelectorAll(".events-tab")) tab.classList.toggle("active", tab.dataset.view === eventsView.mode);
  const body = $("events-body");
  body.replaceChildren();
  if (eventsView.mode === "calendar") {
    body.appendChild(renderEventCalendar(server));
    if (eventsView.day) {
      const day = eventsOnDay(server, eventsView.day);
      body.appendChild(el("h3", "events-day-title", eventsView.day.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" })));
      if (!day.length) body.appendChild(el("div", "manage-empty", "Nothing scheduled"));
      for (const event of day) body.appendChild(eventCard(server, event));
    }
    return;
  }
  const now = Date.now();
  const upcoming = (server.events || []).filter(e => new Date(e.end || e.start).getTime() >= now);
  if (!upcoming.length) {
    const empty = el("div", "events-empty");
    empty.appendChild(svgIcon("calendar", 40));
    empty.appendChild(el("h3", null, "No upcoming events"));
    empty.appendChild(el("p", null, canCreateEvents(server) ? "Schedule one so people know when to show up." : "Check back later."));
    body.appendChild(empty);
    return;
  }
  for (const event of upcoming) body.appendChild(eventCard(server, event));
}

function sameDay(a, b) {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

function eventsOnDay(server, day) {
  return (server.events || []).filter(e => sameDay(new Date(e.start), day));
}

function renderEventCalendar(server) {
  const wrap = el("div", "cal");
  const month = eventsView.month;
  const head = el("div", "cal-head");
  const prev = el("button", "icon-btn");
  prev.title = "Previous month";
  prev.appendChild(svgIcon("chevronLeft", 18));
  prev.onclick = () => {
    eventsView.month = new Date(month.getFullYear(), month.getMonth() - 1, 1);
    renderEvents();
  };
  const next = el("button", "icon-btn");
  next.title = "Next month";
  next.appendChild(svgIcon("chevronRight", 18));
  next.onclick = () => {
    eventsView.month = new Date(month.getFullYear(), month.getMonth() + 1, 1);
    renderEvents();
  };
  head.append(prev, el("span", "cal-month", month.toLocaleDateString([], { month: "long", year: "numeric" })), next);
  wrap.appendChild(head);
  const grid = el("div", "cal-grid");
  for (const d of ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]) grid.appendChild(el("div", "cal-dow", d));
  const start = new Date(month.getFullYear(), month.getMonth(), 1 - month.getDay());
  const today = new Date();
  for (let i = 0; i < 42; i++) {
    const day = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i);
    const cell = el("button", "cal-day");
    if (day.getMonth() !== month.getMonth()) cell.classList.add("other");
    if (sameDay(day, today)) cell.classList.add("today");
    if (eventsView.day && sameDay(day, eventsView.day)) cell.classList.add("selected");
    cell.appendChild(el("span", "cal-num", String(day.getDate())));
    const events = eventsOnDay(server, day);
    for (const event of events.slice(0, 2)) cell.appendChild(el("span", "cal-event", event.title));
    if (events.length > 2) cell.appendChild(el("span", "cal-more", `+${events.length - 2} more`));
    cell.onclick = () => {
      eventsView.day = day;
      renderEvents();
    };
    grid.appendChild(cell);
  }
  wrap.appendChild(grid);
  return wrap;
}

function formatEventTime(event) {
  const start = new Date(event.start);
  const opts = { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" };
  let text = start.toLocaleString([], opts);
  if (event.end) {
    const end = new Date(event.end);
    text += sameDay(start, end)
      ? ` - ${end.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`
      : ` - ${end.toLocaleString([], opts)}`;
  }
  return text;
}

function eventCard(server, event) {
  const card = el("div", "event-card");
  const start = new Date(event.start);
  const live = start.getTime() <= Date.now() && (!event.end || new Date(event.end).getTime() > Date.now());
  const badge = el("div", "event-date");
  badge.appendChild(el("span", "event-date-month", start.toLocaleDateString([], { month: "short" }).toUpperCase()));
  badge.appendChild(el("span", "event-date-day", String(start.getDate())));
  card.appendChild(badge);
  const main = el("div", "event-main");
  const when = el("div", "event-when" + (live ? " live" : ""), live ? "Happening now" : formatEventTime(event));
  main.appendChild(when);
  main.appendChild(el("div", "event-title", event.title));
  if (event.channel_name) {
    const where = el("div", "event-where");
    where.appendChild(svgIcon("speaker", 14));
    where.appendChild(el("span", null, event.channel_name));
    main.appendChild(where);
  }
  if (event.description) main.appendChild(renderMarkdown(event.description));
  const foot = el("div", "event-foot");
  foot.appendChild(el("span", "event-counts", `${event.going} going - ${event.interested} interested - by ${event.creator_name}`));
  const rsvp = (value, label) => {
    const btn = el("button", "btn-small " + (event.my_rsvp === value ? "btn-primary" : "btn-secondary"), label);
    btn.onclick = () => setRsvp(event, event.my_rsvp === value ? null : value);
    return btn;
  };
  const actions = el("div", "event-actions");
  actions.append(rsvp("interested", "Interested"), rsvp("going", "Going"));
  if (event.channel && live) {
    const join = el("button", "btn-primary btn-small", "Join Voice");
    join.onclick = () => {
      const channel = server.channels.find(c => c.id === event.channel);
      closeEvents();
      if (channel && state.voiceChannelId !== channel.id) toggleVoice(channel);
    };
    actions.appendChild(join);
  }
  if (event.can_edit) {
    const edit = el("button", "icon-btn");
    edit.title = "Edit Event";
    edit.appendChild(svgIcon("pencil", 16));
    edit.onclick = () => openEventEditor(event);
    const del = el("button", "icon-btn danger");
    del.title = "Delete Event";
    del.appendChild(svgIcon("trash", 16));
    del.onclick = () => deleteEvent(event);
    actions.append(edit, del);
  }
  foot.appendChild(actions);
  main.appendChild(foot);
  card.appendChild(main);
  return card;
}

async function setRsvp(event, rsvp) {
  try {
    await invoke("set_rsvp", { serverId: state.currentServer.id, eventId: event.id, rsvp });
    await refreshCurrentServer();
    renderEvents();
  } catch (e) {
    toast(String(e));
  }
}

async function deleteEvent(event) {
  if (!confirm(`Delete the event "${event.title}"?`)) return;
  try {
    await invoke("delete_event", { serverId: state.currentServer.id, eventId: event.id });
    await refreshCurrentServer();
    renderEvents();
  } catch (e) {
    toast(String(e));
  }
}

function toLocalInput(date) {
  const pad = n => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function fillVoiceChannelSelect(select, noneLabel, server, value) {
  select.replaceChildren(new Option(noneLabel, ""), ...server.channels.filter(c => c.kind === "voice").map(c => new Option(c.name, c.id)));
  select.value = value;
}

function openEventEditor(event) {
  const server = state.currentServer;
  editingEventId = event ? event.id : null;
  $("event-editor-title").textContent = event ? "Edit Event" : "Create Event";
  $("event-title").value = event ? event.title : "";
  $("event-description").value = event ? event.description : "";
  const start = event ? new Date(event.start) : new Date(Math.ceil(Date.now() / 3600e3) * 3600e3 + 3600e3);
  $("event-start").value = toLocalInput(start);
  $("event-end").value = event && event.end ? toLocalInput(new Date(event.end)) : "";
  fillVoiceChannelSelect($("event-channel"), "No voice channel", server, event && event.channel ? event.channel : "");
  showModal("modal-event");
  $("event-title").focus();
}

async function saveEvent() {
  const startValue = $("event-start").value;
  const endValue = $("event-end").value;
  if (!$("event-title").value.trim() || !startValue) {
    toast("Events need a topic and a start time");
    return;
  }
  try {
    await invoke("save_event", {
      serverId: state.currentServer.id,
      eventId: editingEventId,
      title: $("event-title").value,
      description: $("event-description").value,
      start: new Date(startValue).toISOString(),
      end: endValue ? new Date(endValue).toISOString() : null,
      channelId: $("event-channel").value || null,
    });
    hideModals();
    await refreshCurrentServer();
    renderEvents();
    toast(editingEventId ? "Event updated" : "Event created");
  } catch (e) {
    toast(String(e));
  }
}

const COLLAPSED_KEY = "commy.collapsedSections";

function collapsedSections() {
  try {
    return new Set(JSON.parse(localStorage.getItem(COLLAPSED_KEY) || "[]"));
  } catch {
    return new Set();
  }
}

function toggleSection(key) {
  const set = collapsedSections();
  if (set.has(key)) set.delete(key);
  else set.add(key);
  try { localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...set])); } catch {}
  renderChannels();
}

function channelSections(server, canCreate) {
  const byPos = (a, b) => isVoiceKind(a.kind) - isVoiceKind(b.kind) || a.position - b.position;
  const afk = server.channels.find(c => c.id === server.afk_channel && c.kind === "voice");
  const channels = server.channels.filter(c => c !== afk);
  const categories = channels.filter(c => c.kind === "category").sort((a, b) => a.position - b.position);
  const catIds = new Set(categories.map(c => c.id));
  const loose = channels.filter(c => c.kind !== "category" && !(c.parent && catIds.has(c.parent)));
  const sections = [];
  const looseText = loose.filter(holdsMessages).sort(byPos);
  const looseVoice = loose.filter(c => isVoiceKind(c.kind)).sort(byPos);
  if (looseText.length || canCreate) sections.push({ key: `${server.id}:text`, title: "Text Channels", kind: "text", parent: null, channels: looseText });
  if (looseVoice.length || canCreate) sections.push({ key: `${server.id}:voice`, title: "Voice Channels", kind: "voice", parent: null, channels: looseVoice });
  for (const category of categories) {
    sections.push({
      key: category.id,
      title: category.name,
      kind: "text",
      parent: category.id,
      category,
      channels: channels.filter(c => c.parent === category.id && c.kind !== "category").sort(byPos),
    });
  }
  if (afk) sections.push({ key: `${server.id}:afk`, title: "AFK", kind: "voice", parent: null, fixed: true, channels: [afk] });
  return sections;
}

let dragChannel = null;

async function dropChannel(section, beforeId) {
  const server = state.currentServer;
  const moving = dragChannel;
  dragChannel = null;
  if (!server || !moving) return;
  if (moving.kind === "category") return dropCategory(beforeId);
  const siblings = section.channels.filter(c => c.id !== moving.id);
  let index = beforeId ? siblings.findIndex(c => c.id === beforeId) : siblings.length;
  if (index < 0) index = siblings.length;
  siblings.splice(index, 0, moving);
  const moves = siblings.map((c, i) => ({ channel: c.id, parent: section.parent, position: i }));
  try {
    await invoke("move_channels", { serverId: server.id, moves });
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

async function dropCategory(beforeId) {
  const server = state.currentServer;
  const moving = server.channels.find(c => c.id === (dragCategoryId || ""));
  dragCategoryId = null;
  if (!moving) return;
  const categories = server.channels.filter(c => c.kind === "category" && c.id !== moving.id).sort((a, b) => a.position - b.position);
  let index = beforeId ? categories.findIndex(c => c.id === beforeId) : categories.length;
  if (index < 0) index = categories.length;
  categories.splice(index, 0, moving);
  const moves = categories.map((c, i) => ({ channel: c.id, parent: null, position: 1000 + i }));
  try {
    await invoke("move_channels", { serverId: server.id, moves });
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

let dragCategoryId = null;

function makeDropTarget(node, onDrop) {
  node.addEventListener("dragover", e => {
    if (!dragChannel && !dragCategoryId) return;
    e.preventDefault();
    node.classList.add("drop-over");
  });
  node.addEventListener("dragleave", () => node.classList.remove("drop-over"));
  node.addEventListener("drop", e => {
    e.preventDefault();
    node.classList.remove("drop-over");
    onDrop();
  });
}

function sectionHeader(section, collapsed, canCreate) {
  const header = el("div", "channel-group dm-group section-head" + (collapsed ? " collapsed" : ""));
  const caret = el("span", "section-caret");
  caret.appendChild(svgIcon("chevronDown", 12));
  header.appendChild(caret);
  header.appendChild(el("span", "section-title", section.title));
  header.onclick = () => toggleSection(section.key);
  if (canCreate) {
    if (section.category) {
      header.appendChild(channelGear(section.category));
      header.draggable = true;
      header.addEventListener("dragstart", e => {
        dragCategoryId = section.category.id;
        dragChannel = { kind: "category", id: section.category.id };
        e.dataTransfer.effectAllowed = "move";
      });
    }
    const add = el("button", "dm-group-add");
    add.title = "Create Channel";
    add.appendChild(svgIcon("plus", 16));
    add.onclick = e => {
      e.stopPropagation();
      openCreateChannel(section.kind, section.parent);
    };
    header.appendChild(add);
    makeDropTarget(header, () => {
      if (dragChannel && dragChannel.kind === "category") {
        dragChannel = null;
        if (section.category) dropCategory(section.category.id);
        else dragCategoryId = null;
      } else {
        dropChannel(section, null);
      }
    });
  }
  return header;
}

function voiceCountLabel(channel) {
  const n = channel.voice_users.length;
  if (channel.user_limit) return `${String(n).padStart(2, "0")}/${String(channel.user_limit).padStart(2, "0")}`;
  return n ? String(n) : "";
}

function voiceFull(channel) {
  return Boolean(channel.user_limit) && channel.voice_users.length >= channel.user_limit
    && channel.id !== state.voiceChannelId && !(state.currentServer && state.currentServer.can_manage_channels);
}

function selfAssignableRoles(server) {
  return (server && server.roles || []).filter(r => r.self_assignable);
}

function isEmojiTag(tag) {
  return !/[A-Za-z0-9]/.test(tag) && [...tag].length <= 4;
}

function soundEmojiNode(value) {
  const custom = parseEmojiToken(value);
  if (custom) return customEmojiImg(custom.name, custom.image, "custom-emoji");
  return el("span", "sound-emoji", value);
}

function botTag() {
  const chip = el("span", "bot-tag", "BOT");
  chip.title = "Bot: runs on its own, not a person";
  return chip;
}

function roleTagChip(tag, color) {
  const custom = parseEmojiToken(tag);
  if (custom) {
    const chip = el("span", "role-tag emoji");
    chip.appendChild(customEmojiImg(custom.name, custom.image, "custom-emoji"));
    return chip;
  }
  if (isEmojiTag(tag)) return el("span", "role-tag emoji", tag);
  const chip = el("span", "role-tag", tag);
  if (color) {
    paintRole(chip, color);
    chip.style.borderColor = `#${color}66`;
  }
  return chip;
}

function paintBanner(node, ref, fallbackColor) {
  node.style.backgroundImage = "";
  node.style.background = fallbackColor;
  if (!ref) return;
  loadServerImage(ref).then(url => {
    if (url) {
      node.style.background = "";
      node.style.backgroundImage = `url("${url}")`;
    }
  });
}

function paintProfileBanner(node, banner, provider, role, fallback) {
  if (!banner || !banner.startsWith("blob:")) {
    paintBanner(node, role, fallback);
    return;
  }
  node.style.backgroundImage = "";
  node.style.background = fallback;
  const blob = banner.slice(5);
  loadAvatarImage(blob, provider).then(url => {
    if (url && node.dataset.banner === blob) {
      node.style.background = "";
      node.style.backgroundImage = `url("${url}")`;
    }
  });
  node.dataset.banner = blob;
}

let friends = { friends: [], incoming: [], outgoing: [] };

async function loadFriends() {
  try {
    friends = await invoke("list_friends");
  } catch {}
}

function friendState(userId) {
  if (friends.friends.some(f => f.id === userId)) return "friend";
  if (friends.incoming.some(f => f.id === userId)) return "incoming";
  if (friends.outgoing.some(f => f.id === userId)) return "outgoing";
  return "none";
}

async function friendAction(userId, action, name, note) {
  try {
    await invoke("friend_action", { userId, action, note: note || null });
    await loadFriends();
    const done = { request: `Friend request sent to ${name}`, accept: `You and ${name} are now friends`, decline: "Request declined", remove: `Removed ${name} from friends` };
    toast(done[action] || "Done");
    if (state.view === "dm") renderDmThreads();
  } catch (e) {
    toast(String(e));
  }
}

function friendButton(person, onDone) {
  const st = friendState(person.id);
  const labels = { none: "Add Friend", outgoing: "Request Sent", incoming: "Accept Friend", friend: "Friends" };
  const btn = el("button", "mc-action" + (st === "friend" ? " on" : ""));
  btn.appendChild(svgIcon(st === "friend" ? "check" : "friendAdd", 16));
  btn.appendChild(el("span", null, labels[st]));
  btn.title = st === "friend" ? "Remove friend" : st === "outgoing" ? "Cancel request" : labels[st];
  btn.onclick = async () => {
    if (st === "none") {
      openAddFriend(person, onDone);
      return;
    }
    const action = st === "incoming" ? "accept" : "remove";
    if (action === "remove" && st === "friend" && !confirm(`Remove ${person.name} from your friends?`)) return;
    await friendAction(person.id, action, person.name);
    if (onDone) onDone();
  };
  return btn;
}

let addFriendTarget = null;

function openAddFriend(person, onDone) {
  closeFloating();
  addFriendTarget = { person, onDone };
  const av = $("add-friend-avatar");
  av.replaceChildren();
  paintAvatar(av, person.id, person.name);
  $("add-friend-username").value = person.username ? `@${person.username}` : person.name;
  $("add-friend-note").value = "";
  renderFriendNoteCounter();
  showModal("modal-add-friend");
  $("add-friend-note").focus();
}

function renderFriendNoteCounter() {
  $("add-friend-count").textContent = friendNoteState($("add-friend-note").value).counter;
}

async function sendAddFriend() {
  const target = addFriendTarget;
  if (!target) return;
  const note = friendNoteState($("add-friend-note").value).text;
  addFriendTarget = null;
  hideModals();
  await friendAction(target.person.id, "request", target.person.name, note);
  if (target.onDone) target.onDone();
}

function wireProfilePrivacy() {
  $("profile-policy").addEventListener("change", saveProfilePrivacy);
  $("add-friend-note").addEventListener("input", renderFriendNoteCounter);
  $("add-friend-note").addEventListener("keydown", e => {
    if (e.key === "Enter") {
      e.preventDefault();
      sendAddFriend();
    }
  });
  $("add-friend-cancel").onclick = () => {
    addFriendTarget = null;
    hideModals();
  };
  $("add-friend-send").onclick = sendAddFriend;
}

function invitableServers(userId) {
  const person = state.people.get(userId);
  const shared = new Set((person ? person.servers : []).map(s => s.id));
  return railServers().filter(s => s.can_invite && !shared.has(s.id));
}

async function openInviteMenu(person, anchor) {
  const menu = $("invite-menu");
  menu.replaceChildren(el("div", "ctx-title", `Invite ${person.name} to`));
  await refreshPeople();
  const servers = invitableServers(person.id);
  for (const server of servers) {
    const btn = el("button", null, server.name);
    btn.onclick = async () => {
      menu.classList.add("hidden");
      try {
        const ticket = await invoke("get_invite", { serverId: server.id });
        await invoke("open_dm", { userId: person.id });
        const sent = await invoke("send_dm", { userId: person.id, content: `Come join me in ${server.name}! ${ticket}` });
        dmSentToast(sent, person.name, `Invited ${person.name} to ${server.name}`);
      } catch (e) {
        toast(String(e));
      }
    };
    menu.appendChild(btn);
  }
  if (!servers.length) menu.appendChild(el("div", "ctx-empty", "They are already in all your servers"));
  const r = anchor.getBoundingClientRect();
  placeMenuAt(menu, r.right + 4, r.top);
}

const TICKET_RE = /\bcommy[a-z2-7]{40,}\b/;

function inviteCard(text) {
  const link = findInviteLink(text);
  if (link) return inviteLinkCard(link);
  const m = TICKET_RE.exec(text || "");
  if (!m) return null;
  const card = el("div", "invite-card");
  card.appendChild(el("div", "invite-card-title", "You've been invited to join a server"));
  const join = el("button", "btn-primary btn-small", "Join");
  join.onclick = async () => {
    join.disabled = true;
    try {
      const summary = await invoke("join_server", { ticket: m[0] });
      await enterJoinedServer(summary);
    } catch (e) {
      join.disabled = false;
      toast(String(e));
    }
  };
  card.appendChild(join);
  return card;
}

async function openFullProfile(member) {
  closeFloating();
  await refreshPeople();
  const person = { ...(state.people.get(member.id) || {}), ...member };
  $("fp-banner").dataset.banner = "";
  paintProfileBanner($("fp-banner"), person.banner, person.avatar_provider, person.role_banner, person.color ? roleGradient(person.color) : bannerColor(person.id));
  const av = $("fp-avatar");
  av.replaceChildren();
  paintAvatar(av, person.id, person.name);
  av.appendChild(el("span", "presence-dot " + (person.presence || "offline")));
  $("fp-name").textContent = person.name;
  paintRole($("fp-name"), person.color);
  $("fp-name").parentElement.querySelector(".role-tag")?.remove();
  if (person.tag) $("fp-name").after(roleTagChip(person.tag, person.tag_color));
  $("fp-username").textContent = [person.username ? `@${person.username}` : "", person.pronouns || ""].filter(Boolean).join(" \u00b7 ");
  setEmojiText($("fp-status"), (isShowingActivity(person) ? statusBesideGame(person) : subLine(person)) || presenceLabel(person));
  showActivityIn($("fp-activity"), person);
  $("fp-about-wrap").classList.toggle("hidden", !person.about);
  setEmojiText($("fp-about"), person.about);
  const servers = (state.people.get(person.id) || {}).servers || [];
  const own = !!state.user && person.id === state.user.id;
  $("fp-mutual-label").classList.toggle("hidden", own);
  $("fp-servers").classList.toggle("hidden", own);
  $("fp-mutual-label").textContent = `Mutual Servers - ${servers.length}`;
  $("fp-servers").replaceChildren(...servers.map(s => {
    const row = el("button", "dm-profile-server");
    row.appendChild(el("span", "server-chip", initials(s.name)));
    row.appendChild(el("span", null, s.name));
    row.onclick = () => {
      hideModals();
      selectServer(s.id).then(phoneShowChat);
    };
    return row;
  }));
  if (!servers.length) $("fp-servers").appendChild(el("div", "dm-preview", "No servers in common right now"));
  const st = friendState(person.id);
  $("fp-friend-wrap").classList.toggle("hidden", st === "none");
  $("fp-friend").textContent = { friend: "You are friends", outgoing: "Friend request sent", incoming: "Wants to be your friend" }[st] || "";
  const buttons = $("fp-buttons");
  buttons.replaceChildren();
  const isMe = state.user && person.id === state.user.id;
  if (!isMe) {
    const msg = el("button", "btn-primary btn-small", "Message");
    msg.onclick = () => {
      hideModals();
      messageUser(person.id, person.name);
    };
    const more = el("button", "icon-btn fp-more");
    more.type = "button";
    more.title = "More";
    more.setAttribute("aria-label", "More options");
    more.appendChild(svgIcon("more", 20));
    more.onclick = e => {
      e.stopPropagation();
      const r = more.getBoundingClientRect();
      openProfileMenu(person, r.right - 220, r.bottom + 6);
    };
    buttons.append(friendButton(person, () => openFullProfile(member)), msg, more);
  }
  const modal = $("modal-profile");
  modal.oncontextmenu = isMe ? null : e => {
    e.preventDefault();
    openProfileMenu(person, e.clientX, e.clientY);
  };
  showModal("modal-profile");
}

function openProfileMenu(person, x, y) {
  closeFloating();
  const menu = $("fp-menu");
  menu.replaceChildren(el("div", "ctx-title", person.name));
  const item = (label, icon, onClick, cls = "") => {
    const btn = el("button", cls);
    btn.appendChild(svgIcon(icon, 18));
    btn.appendChild(document.createTextNode(label));
    btn.onclick = () => {
      closeFloating();
      onClick();
    };
    menu.appendChild(btn);
  };
  if (friendState(person.id) === "friend") {
    const wants = wantsFriendOnline(notifyPrefs(), person.id);
    item("Notify when online", "signal", () => toggleFriendOnlinePick(person.id), "ctx-check" + (wants ? " checked" : ""));
  }
  const blocked = isBlocked(person.id);
  item(blocked ? "Unblock" : "Block", "block", () => {
    if (blocked) setBlocked(person.id, person.name, false);
    else {
      hideModals();
      openBlock(person.id, person.name);
    }
  });
  const targets = reportTargetsFor(person.id);
  if (targets.length) {
    item("Report", "flag", () => {
      hideModals();
      openReport({ serverId: targets[0].id, servers: targets, userId: person.id, name: person.name });
    }, "danger");
  }
  placeMenuAt(menu, x, y);
}

let gifTarget = null;

function openMemberCard(member, anchor) {
  const server = state.currentServer || { roles: [] };
  closeFloating();
  const card = $("member-card");
  $("mc-banner").dataset.banner = "";
  paintProfileBanner($("mc-banner"), member.banner, member.avatar_provider, member.role_banner, member.color ? roleGradient(member.color) : bannerColor(member.id));
  const av = $("mc-avatar");
  av.replaceChildren();
  paintAvatar(av, member.id, member.name);
  av.appendChild(el("span", "presence-dot " + (member.presence || "offline")));
  const name = $("mc-name");
  name.textContent = member.name;
  paintRole(name, member.color);
  name.parentElement.querySelectorAll(".role-tag, .bot-tag, .mc-pronouns, .timeout-badge").forEach(n => n.remove());
  if (timeoutActive(member.timeout_until)) name.after(timeoutBadge(member.timeout_until));
  if (member.tag) name.after(roleTagChip(member.tag, member.tag_color));
  if (member.bot) name.after(botTag());
  if (member.pronouns) name.after(el("span", "mc-pronouns", member.pronouns));
  const accountLine = nicknameAccountLine(member);
  $("mc-username").textContent = accountLine || "";
  $("mc-username").classList.toggle("hidden", !accountLine);
  const status = $("mc-status");
  status.replaceChildren();
  const playing = isShowingActivity(member);
  if (playing) {
    setEmojiText(status, statusBesideGame(member) || presenceLabel(member));
  } else {
    const line = subLine(member);
    const cover = member.activity && igdbCover(member.activity_cover, "t_thumb");
    if (cover) {
      const img = el("img", "activity-cover");
      img.src = cover;
      status.appendChild(img);
    }
    appendEmojiText(status, line || presenceLabel(member));
    if (member.activity && member.activity_since) {
      status.appendChild(document.createTextNode(" · "));
      status.appendChild(liveElapsed(member.activity_since, "words"));
    }
  }
  showActivityIn($("mc-activity"), member, () => keepOnScreen(card));
  const roles = $("mc-roles");
  roles.replaceChildren();
  const held = (server.roles || []).filter(r => (member.roles || []).includes(r.id));
  for (const role of held) {
    const chip = el("span", "mc-role");
    const dot = el("span", "role-dot");
    if (role.color) dot.style.background = `#${role.color}`;
    chip.append(dot, document.createTextNode(role.name));
    roles.appendChild(chip);
  }
  $("mc-roles-label").classList.toggle("hidden", !held.length);
  const isMe = state.user && member.id === state.user.id;
  $("mc-about").classList.toggle("hidden", !member.about);
  setEmojiText($("mc-about"), member.about);
  const actions = $("mc-actions");
  actions.replaceChildren();
  const more = $("mc-more");
  more.classList.toggle("hidden", isMe);
  more.replaceChildren(svgIcon("more", 18));
  more.onclick = e => {
    e.stopPropagation();
    const r = more.getBoundingClientRect();
    openProfileMenu(member, r.right - 220, r.bottom + 6);
  };
  const action = (icon, label, onClick, cls = "") => {
    const btn = el("button", "mc-action " + cls);
    btn.appendChild(svgIcon(icon, 16));
    btn.appendChild(el("span", null, label));
    btn.onclick = e => onClick(e);
    actions.appendChild(btn);
    return btn;
  };
  if (!isMe && !member.bot) {
    actions.appendChild(friendButton(member, () => openMemberCard(member, anchor)));
    action("serverAdd", "Invite", e => openInviteMenu(member, e.currentTarget));
  }
  action("person", "Full Profile", () => openFullProfile(member));
  if (state.view === "server" && canTimeOut(state.currentServer, member, state.user && state.user.id)) {
    const active = timeoutActive(currentTimeoutOf(member));
    action("clock", timeoutActionLabel(member), () => (active ? clearMemberTimeout(member) : openTimeoutPicker(member)), active ? "" : "danger");
  }
  if (isMe && canChangeOwnNickname()) action("pencil", "Nickname", () => openNicknameModal(currentServerMember(member.id)));
  $("mc-composer").classList.toggle("hidden", isMe || member.bot || isBlocked(member.id));
  const input = $("mc-input");
  input.value = "";
  input.placeholder = `Message @${member.name}`;
  input.dataset.user = member.id;
  input.dataset.name = member.name;
  card.classList.remove("hidden");
  const a = anchor.getBoundingClientRect();
  const r = card.getBoundingClientRect();
  card.style.left = `${Math.max(8, a.left - r.width - 12)}px`;
  card.style.top = `${Math.max(8, Math.min(a.top, window.innerHeight - r.height - 8))}px`;
}

function decorateAuthor(node, authorId) {
  const member = state.view === "server" && currentServerMember(authorId);
  if (!member) return;
  paintRole(node, member.color);
  if (member.tag) node.after(roleTagChip(member.tag, member.tag_color));
  if (member.bot) node.after(botTag());
}

function openChooseRoles() {
  const server = state.currentServer;
  if (!server) return;
  closeFloating();
  renderRolePicker(server, $("role-pick-list"));
  showModal("modal-choose-roles");
}

function renderRolePicker(server, list) {
  const me = server.members.find(m => state.user && m.id === state.user.id);
  const held = new Set(me ? me.roles : []);
  list.replaceChildren();
  for (const role of selfAssignableRoles(server)) {
    const row = el("button", "role-pick" + (held.has(role.id) ? " held" : "") + (role.banner ? " has-banner" : ""));
    if (role.banner) {
      const strip = el("span", "role-pick-banner");
      paintBanner(strip, role.banner, role.color ? `#${role.color}` : "var(--bg-active)");
      row.appendChild(strip);
    }
    const dot = el("span", "role-dot");
    if (role.color) dot.style.background = `#${role.color}`;
    row.appendChild(dot);
    const name = el("span", "role-pick-name", role.name);
    paintRole(name, role.color);
    row.appendChild(name);
    if (role.tag) row.appendChild(roleTagChip(role.tag, role.color));
    row.appendChild(el("span", "role-pick-check")).appendChild(svgIcon("check", 16));
    row.onclick = async () => {
      const want = !row.classList.contains("held");
      try {
        await invoke("set_own_role", { serverId: server.id, roleId: role.id, held: want });
        row.classList.toggle("held", want);
        await refreshCurrentServer();
      } catch (e) {
        toast(String(e));
      }
    };
    list.appendChild(row);
  }
  if (!list.childElementCount) list.appendChild(el("div", "manage-empty", "This server has no roles to pick yet"));
}

async function uploadRoleBanner(server, role) {
  try {
    const src = await chooseImage("Choose a role banner", $("server-image-file"));
    if (!src) return;
    const data = await cropImageFile(src, 600, 240);
    await invoke("set_role_banner", { serverId: server.id, roleId: role.id, data });
    await refreshCurrentServer();
    openServerSettings();
    toast("Role banner saved");
  } catch (err) {
    toast(String(err));
  }
}

async function removeRoleBanner(server, role) {
  try {
    await invoke("set_role_banner", { serverId: server.id, roleId: role.id, data: null });
    await refreshCurrentServer();
    openServerSettings();
  } catch (err) {
    toast(String(err));
  }
}

const ROLE_COLORS = ["ff4d4d", "ff8a4d", "5fd08a", "3dc4b8", "4d9bff", "7b6bff", "c05ccf", "ff6fa8", "99aab5"];

const PERMISSION_GROUPS = [
  ["Server", ["manage_server", "manage_channels", "manage_roles", "manage_expressions", "create_events", "manage_events", "view_audit_log"]],
  ["Chat", ["manage_messages", "pin_messages", "attach_files", "send_voice_messages", "send_tts_messages", "embed_links", "read_message_history", "create_threads", "send_polls", "post_announcements", "mention_everyone", "bypass_slow_mode", "create_invites"]],
  ["Members", ["kick_members", "ban_members", "mute_members", "time_out_members", "move_members", "manage_nicknames", "change_nickname"]],
  ["Voice", ["priority_speaker"]],
];

function roleForm(server, role, onClose) {
  const creating = !role;
  const form = el("div", "role-form");
  let color = role && role.color ? role.color : ROLE_COLORS[0];
  let tagValue = role ? role.tag || null : null;
  let bannerChange;

  const preview = el("div", "rf-preview");
  const previewName = el("div", "rf-preview-name");
  const tools = el("div", "role-banner-tools");
  const bannerBtn = el("button", "role-banner-btn");
  bannerBtn.type = "button";
  const bannerClear = el("button", "role-banner-btn icon");
  bannerClear.type = "button";
  bannerClear.title = "Remove banner";
  bannerClear.appendChild(svgIcon("close", 14));
  tools.append(bannerClear, bannerBtn);
  preview.append(tools, previewName);

  const name = el("input");
  name.type = "text";
  name.maxLength = 50;
  name.value = role ? role.name : "";
  name.placeholder = "Role name";

  const hasBanner = () => bannerChange ? true : bannerChange === undefined && Boolean(role && role.banner);
  const paintPreview = () => {
    const fallback = `#${color}`;
    if (bannerChange) {
      preview.style.background = "";
      preview.style.backgroundImage = `url("data:image/jpeg;base64,${bannerChange}")`;
    } else if (bannerChange === undefined && role && role.banner) {
      paintBanner(preview, role.banner, fallback);
    } else {
      preview.style.backgroundImage = "";
      preview.style.background = fallback;
    }
    bannerBtn.replaceChildren(svgIcon("plus", 14), el("span", null, hasBanner() ? "Change banner" : "Add banner"));
    bannerClear.classList.toggle("hidden", !hasBanner());
    previewName.replaceChildren();
    if (tagValue) previewName.appendChild(roleTagChip(tagValue, color));
    previewName.appendChild(el("span", null, name.value.trim() || "Role name"));
  };
  bannerBtn.onclick = async () => {
    try {
      const src = await chooseImage("Choose a role banner", $("server-image-file"));
      if (!src) return;
      bannerChange = await cropImageFile(src, 600, 240);
      paintPreview();
    } catch (err) {
      toast(String(err));
    }
  };
  bannerClear.onclick = () => {
    bannerChange = null;
    paintPreview();
  };
  form.appendChild(preview);

  const body = el("div", "rf-body");
  const nameWrap = el("span", "emoji-slot-field");
  const tagBtn = el("button", "emoji-slot");
  tagBtn.type = "button";
  const paintTag = () => {
    tagBtn.replaceChildren(tagValue ? roleTagChip(tagValue, color) : svgIcon("smilePlus", 20));
    tagBtn.classList.toggle("set", Boolean(tagValue));
    tagBtn.title = tagValue ? "Change the role emoji" : "Add an emoji shown next to members with this role";
    paintPreview();
  };
  tagBtn.onclick = e => openEmojiPop(e.currentTarget, emoji => {
    tagValue = emoji;
    paintTag();
  }, {
    custom: true,
    onClear: tagValue ? () => {
      tagValue = null;
      paintTag();
    } : null,
  });
  name.addEventListener("input", paintPreview);
  nameWrap.append(tagBtn, name);
  body.appendChild(el("div", "setting-title", "Name"));
  body.appendChild(nameWrap);
  body.appendChild(el("div", "setting-sub rf-name-sub", "The emoji shows next to the names of members with this role."));

  body.appendChild(el("div", "setting-title", "Colour"));
  const swatches = el("div", "rf-swatches");
  const custom = el("input");
  custom.type = "color";
  custom.title = "Pick any colour";
  const paintSwatches = () => {
    for (const swatch of swatches.querySelectorAll(".rf-swatch")) swatch.classList.toggle("on", swatch.dataset.color === color);
    custom.value = `#${color}`;
  };
  for (const value of ROLE_COLORS) {
    const swatch = el("button", "rf-swatch");
    swatch.type = "button";
    swatch.dataset.color = value;
    swatch.style.background = `#${value}`;
    swatch.setAttribute("aria-label", `Colour ${value}`);
    swatch.onclick = () => {
      color = value;
      paintSwatches();
      paintPreview();
    };
    swatches.appendChild(swatch);
  }
  custom.oninput = () => {
    color = custom.value.replace("#", "");
    paintSwatches();
    paintPreview();
  };
  swatches.appendChild(custom);
  body.appendChild(swatches);


  const labels = new Map(PERMISSION_LABELS);
  const held = new Set(role ? role.permissions : []);
  const permInputs = [];
  const permRow = (value, cls) => {
    const wrap = el("label", cls);
    const input = el("input");
    input.type = "checkbox";
    input.value = value;
    input.checked = switchOn(held, value);
    permInputs.push(input);
    const text = el("span", "perm-text");
    text.appendChild(el("span", "perm-name", labels.get(value)));
    const hint = permissionHint(value);
    if (hint && cls !== "rf-admin") text.appendChild(el("span", "perm-hint", hint));
    wrap.append(input, text);
    return wrap;
  };
  body.appendChild(el("div", "setting-title role-perms-title", "Permissions"));
  for (const [title, values] of PERMISSION_GROUPS) {
    body.appendChild(el("div", "rf-group", title));
    const grid = el("div", "perm-grid");
    for (const value of values) grid.appendChild(permRow(value));
    body.appendChild(grid);
  }
  const admin = permRow("administrator", "rf-admin");
  admin.appendChild(el("span", "setting-sub", "Everything above, and nothing can be refused. Give it to as few people as you can."));
  body.appendChild(admin);

  const pick = el("label", "role-self");
  const check = el("input");
  check.type = "checkbox";
  check.checked = role ? role.self_assignable : false;
  pick.append(check, el("span", null, "Members can pick this role themselves (Choose Roles)"));
  body.appendChild(pick);

  const separate = el("label", "role-self");
  const hoistCheck = el("input");
  hoistCheck.type = "checkbox";
  hoistCheck.checked = role ? !!role.hoist : false;
  separate.append(hoistCheck, el("span", null, "Show members with this role in their own group in the people list"));
  body.appendChild(separate);

  const pingRow = el("label", "rf-mention");
  const pingText = el("div", "rf-mention-text");
  pingText.append(
    el("div", "setting-title", "Allow anyone to mention this role"),
    el("div", "setting-sub", "Off means only members allowed to mention everyone can notify the people holding it."),
  );
  const pingCheck = el("input", "switch");
  pingCheck.type = "checkbox";
  pingCheck.checked = role ? Boolean(role.mentionable) : false;
  pingRow.append(pingText, pingCheck);
  body.appendChild(pingRow);

  const actions = el("div", "form-actions");
  const save = el("button", "btn-primary btn-small", creating ? "Create Role" : "Save Role");
  save.onclick = async () => {
    if (!name.value.trim()) {
      toast("Give the role a name");
      name.focus();
      return;
    }
    const permissions = namesToSave(permInputs.map(input => [input.value, input.checked]));
    try {
      let roleId = role && role.id;
      if (creating) {
        roleId = await invoke("create_role", {
          serverId: server.id,
          name: name.value,
          color,
          permissions,
          tag: tagValue,
          selfAssignable: check.checked,
          hoist: hoistCheck.checked,
          mentionable: pingCheck.checked,
        });
      } else {
        await invoke("update_role", {
          serverId: server.id,
          roleId,
          name: name.value,
          color,
          selfAssignable: check.checked,
          hoist: hoistCheck.checked,
          tag: tagValue,
          permissions,
          mentionable: pingCheck.checked,
        });
      }
      if (bannerChange !== undefined && !(creating && bannerChange === null)) {
        await invoke("set_role_banner", { serverId: server.id, roleId, data: bannerChange });
      }
      if (creating) onClose();
      await refreshCurrentServer();
      openServerSettings();
      toast(creating ? "Role created" : "Role saved");
    } catch (e) {
      toast(String(e));
    }
  };
  if (creating) {
    const cancel = el("button", "btn-secondary btn-small", "Cancel");
    cancel.onclick = onClose;
    actions.append(cancel, save);
  } else {
    const del = el("button", "btn-danger btn-small", "Delete Role");
    del.onclick = async () => {
      if (!confirm(`Delete role ${role.name}?`)) return;
      try {
        await invoke("delete_role", { serverId: server.id, roleId: role.id });
        await refreshCurrentServer();
        openServerSettings();
      } catch (e) {
        toast(String(e));
      }
    };
    actions.append(del, save);
  }
  body.appendChild(actions);
  form.appendChild(body);
  paintSwatches();
  paintTag();
  return form;
}

let safety = { blocked: [], dm_policy: "everyone", spam_filter: true, profile_policy: "everyone" };
const revealedBlocked = new Set();

async function loadSafety() {
  try {
    safety = await invoke("get_safety");
  } catch {}
}

function isBlocked(userId) {
  return Boolean(userId) && safety.blocked.some(b => b.id === userId);
}

const KEEP_LABELS = { 30: "30 days", 90: "90 days", 180: "6 months", 365: "1 year", forever: "Forever" };
let storageSettings = { keep: "90", servers: {} };

async function loadStorageSettings() {
  try {
    storageSettings = await invoke("get_storage_settings");
  } catch {}
}

function keepSelect(value, defaultLabel) {
  const select = el("select", "compact-select");
  for (const [choice, label] of [["", defaultLabel], ...Object.entries(KEEP_LABELS)]) {
    const option = el("option", null, label);
    option.value = choice;
    select.appendChild(option);
  }
  select.value = value;
  return select;
}

function renderStorageSettings() {
  $("keep-default").value = storageSettings.keep;
  const list = $("keep-servers");
  list.replaceChildren();
  for (const server of railServers()) {
    const row = el("div", "keep-row");
    row.appendChild(el("span", "keep-name", server.name));
    const select = keepSelect(storageSettings.servers[server.id] || "", `Default (${KEEP_LABELS[storageSettings.keep]})`);
    select.setAttribute("aria-label", `Keep messages from ${server.name} for`);
    select.onchange = () => setHistoryWindow(server.id, select.value || null);
    row.appendChild(select);
    list.appendChild(row);
  }
  if (!railServers().length) list.appendChild(el("div", "manage-empty", "You are not in any servers yet"));
}

async function setHistoryWindow(serverId, keep) {
  try {
    storageSettings = await invoke("set_history_window", { serverId, keep });
  } catch (e) {
    toast(String(e));
  }
  renderStorageSettings();
}

function renderProfilePrivacy() {
  $("profile-policy").value = safety.profile_policy || "everyone";
}

async function saveProfilePrivacy() {
  try {
    await invoke("set_profile_privacy", { policy: $("profile-policy").value });
    await loadSafety();
  } catch (e) {
    toast(String(e));
  }
  renderProfilePrivacy();
}

function renderSafetySettings() {
  renderProfilePrivacy();
  $("dm-policy").value = safety.dm_policy;
  $("spam-filter-toggle").checked = safety.spam_filter;
  const list = $("blocked-list");
  list.replaceChildren();
  for (const person of safety.blocked) {
    const row = el("div", "blocked-row");
    row.appendChild(makeAvatar(person.id, person.name, "avatar avatar-sm"));
    row.appendChild(el("span", "blocked-name", person.name));
    const unblock = el("button", "btn-secondary btn-small", "Unblock");
    unblock.onclick = () => setBlocked(person.id, person.name, false);
    row.appendChild(unblock);
    list.appendChild(row);
  }
  if (!safety.blocked.length) list.appendChild(el("div", "manage-empty", "You have not blocked anyone"));
}

async function saveDmSettings() {
  try {
    await invoke("set_dm_settings", { policy: $("dm-policy").value, spamFilter: $("spam-filter-toggle").checked });
    await loadSafety();
  } catch (e) {
    toast(String(e));
  }
}

let blockTarget = null;

function openBlock(userId, name) {
  blockTarget = { userId, name };
  $("block-title").textContent = `Block ${name}?`;
  showModal("modal-block");
}

async function setBlocked(userId, name, blocked) {
  try {
    await invoke("set_blocked", { userId, blocked });
    await loadSafety();
    toast(blocked ? `Blocked ${name}` : `Unblocked ${name}`);
    afterSafetyChange();
  } catch (e) {
    toast(String(e));
  }
}

async function afterSafetyChange() {
  if (!$("user-settings").classList.contains("hidden")) renderSafetySettings();
  if (state.view === "dm") {
    if (state.currentDm && isBlocked(state.currentDm.user)) {
      state.currentDm = null;
      await enterDmMode();
    } else {
      await renderDmThreads();
    }
  } else if (state.currentChannel) {
    renderMessages(state.renderedMessages || []);
  }
}

function blockedPlaceholder(run) {
  const row = el("div", "msg-hidden blocked");
  row.appendChild(svgIcon("block", 14));
  row.appendChild(el("span", null, run.length === 1 ? "1 blocked message" : `${run.length} blocked messages`));
  const show = el("button", "btn-link", "Show");
  show.onclick = () => {
    for (const m of run) revealedBlocked.add(m.id);
    renderMessages(state.renderedMessages || []);
  };
  row.appendChild(show);
  return row;
}

function reportServersFor(userId) {
  const person = state.people.get(userId);
  return (person ? person.servers : [])
    .map(s => state.servers.find(x => x.id === s.id) || s)
    .filter(s => s && s.id);
}

function reportTargetsFor(userId) {
  return state.currentServer && reportableIn(state.currentServer, userId)
    ? [state.currentServer]
    : reportServersFor(userId).filter(s => reportableIn(s, userId));
}

function contextMenuBuilder(menu) {
  const item = (label, onClick, cls = "") => {
    const btn = el("button", cls, label);
    btn.onclick = () => {
      closeFloating();
      onClick();
    };
    menu.appendChild(btn);
    return btn;
  };
  const sep = () => menu.appendChild(el("div", "ctx-sep"));
  const slider = (label, value, onChange) => {
    const wrap = el("div", "ctx-slider");
    const text = el("div", "ctx-slider-label");
    text.append(el("span", null, label), el("span", null, `${value}%`));
    const range = el("input");
    range.type = "range";
    range.min = "0";
    range.max = "200";
    range.value = String(value);
    range.oninput = () => {
      text.lastChild.textContent = `${range.value}%`;
      syncRangeFill(range);
    };
    range.onchange = () => onChange(Number(range.value) / 100);
    wrap.append(text, range);
    menu.appendChild(wrap);
    syncRangeFill(range);
  };
  return { item, sep, slider };
}

function openMemberMenu(member, x, y, { dm = false } = {}) {
  if (state.user && member.id === state.user.id) return;
  closeFloating();
  const server = dm ? null : state.currentServer;
  const inServer = Boolean(server && server.members.some(m => m.id === member.id));
  const menu = $("member-menu");
  menu.replaceChildren(el("div", "ctx-title", member.name));
  const { item, sep, slider } = contextMenuBuilder(menu);

  item("Profile", () => openMemberCard(fullMember(member), { getBoundingClientRect: () => ({ left: x, top: y }) }));
  if (!(dm && state.currentDm && state.currentDm.user === member.id)) {
    item("Message", () => messageUser(member.id, member.name));
  }
  if (server && (server.screen_shares || []).some(s => s.user === member.id)) {
    const watching = state.watchingUser === member.id;
    item(watching ? "Stop Watching" : "Watch Stream", () => (watching ? stopWatching() : watchScreen(member.id, member.name)));
  }
  if (dm && (state.dmThreads || []).some(t => t.user === member.id)) {
    const pinned = dmIsPinned(member.id);
    item(pinned ? "Unpin" : "Pin", () => setDmPinned(member.id, !pinned));
  }
  if (friendState(member.id) === "friend") {
    const wants = wantsFriendOnline(notifyPrefs(), member.id);
    const ping = item("Notify when online", () => toggleFriendOnlinePick(member.id), "ctx-check" + (wants ? " checked" : ""));
    ping.title = "A toast when they come online";
  }
  sep();

  const voice = state.voice || {};
  const mutedForMe = (voice.muted_users || []).includes(member.id);
  const mute = item(mutedForMe ? "Unmute" : "Mute", () => setUserVoice(member.id, { muted: !mutedForMe }), "ctx-check" + (mutedForMe ? " checked" : ""));
  mute.title = "Only you stop hearing them";
  const volume = Math.round(((voice.user_volumes || {})[member.id] ?? 1) * 100);
  slider("User Volume", volume, v => setUserVoice(member.id, { volume: v }));
  sep();

  item(isBlocked(member.id) ? "Unblock" : "Block", () => {
    if (isBlocked(member.id)) setBlocked(member.id, member.name, false);
    else openBlock(member.id, member.name);
  });
  const reportTargets = dm
    ? reportServersFor(member.id).filter(s => reportableIn(s, member.id))
    : (reportableIn(server, member.id) ? [server] : []);
  if (reportTargets.length) {
    item("Report", () => openReport({ serverId: reportTargets[0].id, servers: reportTargets, userId: member.id, name: member.name }), "danger");
  }

  const moderates = inServer && (server.can_mute || server.can_move || server.can_kick || server.can_ban || server.can_time_out || server.can_manage_nicknames || server.can_manage_stage);
  if (inServer && member.id !== server.owner && moderates) {
    sep();
    const stageAction = stageMenuAction(stageChannelOf(server, member.id), member.id, Boolean(server.can_manage_stage));
    if (stageAction) {
      const entry = item(stageAction.label, () => setStageSpeaker(stageChannelOf(server, member.id), member, stageAction));
      if (stageAction.speaker && (stageChannelOf(server, member.id).hands || []).includes(member.id)) {
        entry.classList.add("ctx-hand");
        entry.prepend(svgIcon("hand", 14));
      }
    }
    if (server.can_mute) {
      const sm = (server.server_muted || []).includes(member.id);
      const sd = (server.server_deafened || []).includes(member.id);
      item("Server Mute", () => moderateVoice(member, !sm, sd), "ctx-check" + (sm ? " checked" : ""));
      item("Server Deafen", () => moderateVoice(member, sm, !sd), "ctx-check" + (sd ? " checked" : ""));
    }
    if (server.can_manage_nicknames) item("Change Nickname", () => openNicknameModal(member));
    if (canTimeOut(server, member, state.user && state.user.id)) {
      const active = timeoutActive(currentTimeoutOf(member));
      item(active ? "Remove timeout" : `Time out ${member.name}`, () => (active ? clearMemberTimeout(member) : openTimeoutPicker(member)), active ? "" : "danger");
    }
    const inVoice = server.can_move ? (server.members.find(m => m.id === member.id) || {}).voice_channel : null;
    if (inVoice) {
      for (const channel of server.channels.filter(c => isVoiceKind(c.kind) && c.id !== inVoice && c.can_view !== false)) {
        item(`Move to ${channelPlainName(channel)}`, () => moveVoiceMember(member, channel));
      }
      item("Disconnect from Voice", () => moveVoiceMember(member, null), "danger");
    }
    if (server.can_kick) item(`Kick ${member.name}`, () => confirmKick(member), "danger");
    if (server.can_ban) item(`Ban ${member.name}`, () => confirmBan(member), "danger");
  }
  placeMenuAt(menu, x, y);
}

let moderateRun = null;

function modWho(target, member) {
  target.replaceChildren(makeAvatar(member.id, member.name, "avatar avatar-sm"), el("span", null, member.name));
}

function askModeration({ title, member, text, label, run }) {
  closeFloating();
  $("moderate-title").textContent = title;
  modWho($("moderate-who"), member);
  $("moderate-text").textContent = text;
  $("moderate-go").textContent = label;
  moderateRun = run;
  showModal("modal-moderate");
}

function confirmKick(member) {
  const server = state.currentServer;
  if (!server) return;
  askModeration({
    title: `Kick ${member.name}`,
    member,
    text: `They are removed from ${server.name} and can rejoin with a new invite.`,
    label: "Kick",
    run: async () => {
      await invoke("kick_member", { serverId: server.id, userId: member.id });
      await refreshCurrentServer();
      toast(`Kicked ${member.name}`);
    },
  });
}

function confirmBan(member) {
  const server = state.currentServer;
  if (!server) return;
  askModeration({
    title: `Ban ${member.name}`,
    member,
    text: `They are removed from ${server.name} and cannot rejoin until you unban them.`,
    label: "Ban",
    run: async () => {
      await invoke("ban_member", { serverId: server.id, userId: member.id });
      await refreshCurrentServer();
      toast(`Banned ${member.name}`);
    },
  });
}

function wireModerationModal() {
  $("moderate-cancel").onclick = () => {
    hideModals();
    moderateRun = null;
  };
  $("moderate-go").onclick = async () => {
    const run = moderateRun;
    moderateRun = null;
    hideModals();
    if (!run) return;
    try {
      await run();
      if (!$("server-settings").classList.contains("hidden")) openServerSettings();
    } catch (e) {
      toast(String(e));
    }
  };
}

function openMemberRowMenu(member, anchor) {
  const server = state.currentServer;
  if (!server) return;
  closeFloating();
  const menu = $("member-menu");
  menu.replaceChildren(el("div", "ctx-title", member.name));
  const { item, sep } = contextMenuBuilder(menu);
  const isSelf = state.user && member.id === state.user.id;
  const isOwner = member.id === server.owner;
  let any = false;
  if (canTimeOut(server, member, state.user && state.user.id)) {
    const active = timeoutActive(member.timeout_until);
    item(active ? "Remove timeout" : "Time out...", async () => {
      if (active) {
        await clearMemberTimeout(member);
        openServerSettings();
      } else {
        openTimeoutPicker(member);
      }
    });
    any = true;
  }
  if (!isSelf && !isOwner && (server.can_kick || server.can_ban)) {
    if (any) sep();
    if (server.can_kick) item("Kick...", () => confirmKick(member), "danger");
    if (server.can_ban) item("Ban...", () => confirmBan(member), "danger");
  }
  const rect = anchor.getBoundingClientRect();
  placeMenuAt(menu, rect.left, rect.bottom + 4);
  const width = menu.getBoundingClientRect().width;
  menu.style.left = `${Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8))}px`;
}

async function setStageSpeaker(channel, member, action) {
  if (!channel || !state.currentServer) return;
  try {
    await invoke("set_stage_speaker", { serverId: state.currentServer.id, channelId: channel.id, userId: member.id, speaker: action.speaker });
    toast(`${member.name} ${action.done}`);
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

async function toggleHand(raised) {
  if (!state.voiceServer) return;
  try {
    await invoke("raise_hand", { serverId: state.voiceServer.id, raised });
    toast(raised ? "Hand raised" : "Hand lowered");
    if (state.currentServer && state.currentServer.id === state.voiceServer.id) await refreshCurrentServer();
    else renderVoicePanel();
  } catch (e) {
    toast(String(e));
  }
}

function openSelfVoiceMenu(x, y) {
  if (!state.user) return;
  closeFloating();
  const menu = $("member-menu");
  menu.replaceChildren(el("div", "ctx-title", state.user.display_name || state.user.username));
  const { item, sep, slider } = contextMenuBuilder(menu);
  const level = (label, key, onChange) => slider(label, Math.round((state.voice[key] ?? 1) * 100), onChange);
  const setLevels = async (outputVolume, inputGain) => {
    try {
      state.voice = await invoke("set_volumes", { outputVolume, inputGain });
    } catch (e) {
      toast(String(e));
    }
  };

  item("Profile", () => openMemberCard(fullMember(state.user), { getBoundingClientRect: () => ({ left: x, top: y }) }));
  if (canChangeOwnNickname()) item("Change Nickname", () => openNicknameModal(currentServerMember(state.user.id)));
  sep();
  item(state.voice.muted ? "Unmute" : "Mute", async () => {
    state.voice = await invoke("set_muted", { muted: !state.voice.muted });
    renderUserPanel();
    renderChannels();
  }, "ctx-check" + (state.voice.muted ? " checked" : ""));
  item(state.voice.deafened ? "Undeafen" : "Deafen", async () => {
    state.voice = await invoke("set_deafened", { deafened: !state.voice.deafened });
    renderUserPanel();
    renderChannels();
  }, "ctx-check" + (state.voice.deafened ? " checked" : ""));
  level("Microphone", "input_gain", v => setLevels(state.voice.output_volume ?? 1, v));
  level("Speakers", "output_volume", v => setLevels(v, state.voice.input_gain ?? 1));
  sep();
  item("Voice Settings", () => openUserSettings("voice"));
  item("Disconnect", () => disconnectVoice(), "danger");
  placeMenuAt(menu, x, y);
}

function fullMember(person) {
  const fromServer = currentServerMember(person.id);
  if (fromServer) return fromServer;
  const known = state.people.get(person.id);
  return { presence: "offline", roles: [], ...(known || {}), ...person };
}

async function setUserVoice(userId, { volume, muted }) {
  try {
    state.voice = await invoke("set_user_voice", { userId, volume: volume ?? null, muted: muted ?? null });
  } catch (e) {
    toast(String(e));
  }
}

function currentTimeoutOf(member) {
  const fresh = currentServerMember(member.id);
  return fresh ? fresh.timeout_until : member.timeout_until;
}

function timeoutBadge(untilMs) {
  const badge = el("span", "timeout-badge");
  badge.dataset.until = String(untilMs);
  badge.title = timeoutBadgeTitle(untilMs);
  badge.setAttribute("aria-label", badge.title);
  badge.appendChild(svgIcon("clock", 12));
  return badge;
}

setInterval(() => {
  for (const node of document.querySelectorAll(".timeout-badge[data-until]")) {
    const until = Number(node.dataset.until);
    if (!timeoutActive(until)) node.remove();
    else node.title = timeoutBadgeTitle(until);
  }
  const left = document.querySelector(".timeout-left[data-until]");
  if (left) {
    const until = Number(left.dataset.until);
    if (timeoutActive(until)) left.textContent = timeoutLeftText(until);
    else syncRulesGate();
  }
}, 1000);

let timeoutTarget = null;

function openTimeoutPicker(member) {
  const server = state.currentServer;
  if (!server || !canTimeOut(server, member, state.user && state.user.id)) return;
  closeFloating();
  timeoutTarget = { serverId: server.id, member, seconds: TIMEOUT_CHOICES[1].seconds };
  $("timeout-title").textContent = `Time out ${member.name}`;
  modWho($("timeout-who"), member);
  $("timeout-custom").value = "";
  renderTimeoutChoices();
  showModal("modal-timeout");
}

function renderTimeoutChoices() {
  const grid = $("timeout-choices");
  grid.replaceChildren();
  const raw = $("timeout-custom").value.trim();
  const custom = customTimeoutSeconds(raw);
  for (const choice of TIMEOUT_CHOICES) {
    const picked = !raw && Boolean(timeoutTarget) && timeoutTarget.seconds === choice.seconds;
    const btn = el("button", "timeout-choice" + (picked ? " active" : ""), choice.label);
    btn.type = "button";
    btn.setAttribute("role", "radio");
    btn.setAttribute("aria-checked", String(picked));
    btn.onclick = () => {
      if (!timeoutTarget) return;
      timeoutTarget.seconds = choice.seconds;
      $("timeout-custom").value = "";
      renderTimeoutChoices();
    };
    grid.appendChild(btn);
  }
  $("timeout-go").disabled = !timeoutTarget || !(custom || (!raw && timeoutTarget.seconds));
}

async function confirmTimeout() {
  if (!timeoutTarget) return;
  const { serverId, member } = timeoutTarget;
  const raw = $("timeout-custom").value.trim();
  const seconds = raw ? customTimeoutSeconds(raw) : timeoutTarget.seconds;
  if (!seconds) return;
  hideModals();
  timeoutTarget = null;
  try {
    await invoke("time_out_member", { serverId, userId: member.id, seconds });
    await refreshCurrentServer();
    toast(timeoutSummary(member.name, seconds));
    if (!$("server-settings").classList.contains("hidden")) openServerSettings();
  } catch (e) {
    toast(String(e));
  }
}

async function moveVoiceMember(member, channel) {
  const serverId = state.currentServer.id;
  const roomName = channel ? channelPlainName(channel) : null;
  try {
    await invoke("move_voice_member", { serverId, userId: member.id, channelId: channel ? channel.id : null });
    await refreshCurrentServer();
    toast(channel ? `Moving ${member.name} to ${roomName}` : `Disconnecting ${member.name} from voice`);
    setTimeout(() => checkVoiceMove(serverId, member, channel ? channel.id : null, roomName), VOICE_MOVE_CHECK_MS);
  } catch (e) {
    toast(String(e));
  }
}

function checkVoiceMove(serverId, member, roomId, roomName) {
  const server = state.currentServer;
  if (!server || server.id !== serverId || voiceMoveLanded(server, member.id, roomId)) return;
  toast(voiceMoveStuckText(member.name, roomName));
}

async function clearMemberTimeout(member) {
  const server = state.currentServer;
  if (!server) return;
  try {
    await invoke("clear_timeout", { serverId: server.id, userId: member.id });
    await refreshCurrentServer();
    toast(`Removed the timeout on ${member.name}`);
  } catch (e) {
    toast(String(e));
  }
}

function wireTimeoutModal() {
  $("timeout-cancel").onclick = () => {
    hideModals();
    timeoutTarget = null;
  };
  $("timeout-go").onclick = confirmTimeout;
  $("timeout-custom").oninput = renderTimeoutChoices;
  $("timeout-custom").onkeydown = e => {
    if (e.key === "Enter" && !$("timeout-go").disabled) confirmTimeout();
  };
}

function canChangeOwnNickname() {
  const server = state.view === "server" ? state.currentServer : null;
  return Boolean(server && state.user && server.can_change_nickname && currentServerMember(state.user.id));
}

let nicknameTarget = null;

function openNicknameModal(member) {
  const server = state.currentServer;
  if (!member || !server) return;
  closeFloating();
  nicknameTarget = { serverId: server.id, userId: member.id };
  $("nickname-hint").textContent = nicknamePrompt(member, server, state.user && state.user.id);
  const input = $("nickname-input");
  input.value = member.nickname || "";
  input.placeholder = member.username || "Nickname";
  $("nickname-clear").disabled = !member.nickname;
  showModal("modal-nickname");
  input.focus();
  input.select();
}

async function saveNickname(nickname) {
  const target = nicknameTarget;
  if (!target) return;
  const problem = nickname === null ? null : nicknameProblem(nickname);
  if (problem) {
    toast(problem);
    return;
  }
  try {
    await invoke("set_nickname", { serverId: target.serverId, userId: target.userId, nickname: nickname === null ? null : cleanNickname(nickname) });
    hideModals();
    nicknameTarget = null;
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

function wireNicknameModal() {
  $("nickname-save").onclick = () => saveNickname($("nickname-input").value);
  $("nickname-clear").onclick = () => saveNickname(null);
  $("nickname-cancel").onclick = hideModals;
  $("nickname-input").addEventListener("keydown", e => {
    if (e.key === "Enter") saveNickname(e.currentTarget.value);
  });
}

async function moderateVoice(member, muted, deafened) {
  try {
    await invoke("moderate_voice", { serverId: state.currentServer.id, userId: member.id, muted, deafened });
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

function reportableIn(server, userId) {
  return Boolean(server && state.user && server.owner !== state.user.id && userId !== server.owner && userId !== state.user.id);
}

let reportTarget = null;

function openReport(target) {
  reportTarget = target;
  const servers = target.servers || [];
  $("report-server-row").classList.toggle("hidden", servers.length < 2);
  const pick = $("report-server");
  pick.replaceChildren(...servers.map(s => new Option(s.name, s.id)));
  pick.value = target.serverId;
  pick.onchange = () => {
    reportTarget.serverId = pick.value;
  };
  $("report-title").textContent = target.messageId ? `Report message from ${target.name}` : `Report ${target.name}`;
  for (const radio of document.querySelectorAll('input[name="report-reason"]')) radio.checked = false;
  $("report-note").value = "";
  $("report-block").checked = false;
  const preview = $("report-preview");
  preview.replaceChildren();
  preview.classList.toggle("hidden", !target.excerpt);
  if (target.excerpt) {
    preview.appendChild(el("div", "msg-author", target.name));
    preview.appendChild(buildMessageContent(target.excerpt, target.userId));
  }
  showModal("modal-report");
}

async function submitReport() {
  const reason = document.querySelector('input[name="report-reason"]:checked');
  if (!reason) {
    toast("Pick a reason for the report");
    return;
  }
  const t = reportTarget;
  const go = $("report-go");
  go.disabled = true;
  try {
    await invoke("report_user", {
      serverId: t.serverId,
      userId: t.userId,
      channelId: t.channelId || null,
      messageId: t.messageId || null,
      reason: reason.value,
      note: $("report-note").value,
    });
    hideModals();
    toast("Report sent to the server owner");
    if ($("report-block").checked) await setBlocked(t.userId, t.name, true);
  } catch (e) {
    toast(String(e));
  } finally {
    go.disabled = false;
  }
}

const REPORT_REASON_LABELS = {
  spam: "Spam or scams",
  harassment: "Harassment or hate",
  inappropriate: "Inappropriate content",
  impersonation: "Impersonation",
  other: "Something else",
};

async function renderReportSettings(server) {
  const list = $("report-list");
  list.replaceChildren();
  let reports = [];
  try {
    reports = await invoke("list_reports", { serverId: server.id });
  } catch (e) {
    list.appendChild(el("div", "manage-empty", String(e)));
    return;
  }
  $("tab-reports").textContent = reports.length ? `Reports (${reports.length})` : "Reports";
  for (const report of reports) {
    const card = el("div", "report-card");
    const head = el("div", "report-head");
    head.appendChild(makeAvatar(report.reported, report.reported_name, "avatar avatar-sm"));
    const who = el("div", "report-who");
    who.appendChild(el("div", "report-name", report.reported_name));
    who.appendChild(el("div", "report-meta", `Reported by ${report.reporter_name} - ${formatShortTime(report.created_at)}`));
    head.appendChild(who);
    head.appendChild(el("span", "report-reason", REPORT_REASON_LABELS[report.reason] || report.reason));
    card.appendChild(head);
    if (report.excerpt) {
      const quote = el("div", "report-excerpt");
      quote.appendChild(buildMessageContent(report.excerpt, report.reported));
      card.appendChild(quote);
    }
    if (report.note) card.appendChild(el("div", "report-note", report.note));
    const actions = el("div", "report-actions");
    if (report.channel && report.message) {
      const jump = el("button", "btn-secondary btn-small", "Jump to Message");
      jump.onclick = async () => {
        await closeSettingsLayers();
        await openNotificationTarget(`channel:${report.server}:${report.channel}`);
        setTimeout(() => jumpToMessage(report.message), 300);
      };
      actions.appendChild(jump);
    }
    const isMember = server.members.some(m => m.id === report.reported);
    if (server.can_kick && isMember) {
      const kick = el("button", "btn-danger btn-small", "Kick");
      kick.onclick = async () => {
        if (!confirm(`Kick ${report.reported_name} from the server?`)) return;
        try {
          await invoke("kick_member", { serverId: server.id, userId: report.reported });
          await refreshCurrentServer();
          toast(`Kicked ${report.reported_name}`);
        } catch (e) {
          toast(String(e));
        }
      };
      actions.appendChild(kick);
    }
    const dismiss = el("button", "btn-link", "Dismiss");
    dismiss.onclick = async () => {
      await invoke("dismiss_report", { reportId: report.id });
      renderReportSettings(state.currentServer);
    };
    actions.appendChild(dismiss);
    card.appendChild(actions);
    list.appendChild(card);
  }
  if (!reports.length) list.appendChild(el("div", "manage-empty", "No reports. All quiet."));
}

function onReportReceived(report) {
  const server = state.servers.find(s => s.id === report.server);
  invoke("notify", {
    title: `New report${server ? ` in ${server.name}` : ""}`,
    body: `${report.reporter_name} reported ${report.reported_name}`,
    target: null,
  }).catch(() => {});
  playSound("message");
  if (!$("server-settings").classList.contains("hidden") && state.currentServer && state.currentServer.id === report.server) {
    renderReportSettings(state.currentServer);
  }
}

function renderDmRequestBar(thread) {
  const isRequest = Boolean(thread && thread.kind !== "inbox");
  $("dm-request-bar").classList.toggle("hidden", !isRequest);
  document.querySelector(".composer").classList.toggle("hidden", isRequest);
  if (!isRequest) return;
  $("dm-request-title").textContent = thread.kind === "spam"
    ? `${thread.name} might be spam`
    : `${thread.name} wants to message you`;
  $("dm-request-sub").textContent = thread.kind === "spam"
    ? "This looked like spam, so you were not notified."
    : "Accept to reply, or decline to delete the request.";
}

async function answerDmRequest(action) {
  const dm = state.currentDm;
  if (!dm) return;
  try {
    if (action === "block") {
      openBlock(dm.user, dm.name);
      return;
    }
    await invoke(action === "accept" ? "accept_dm_request" : "decline_dm_request", { userId: dm.user });
    if (action === "accept") {
      await selectDm(dm.user, dm.name);
    } else {
      state.currentDm = null;
      await enterDmMode();
    }
  } catch (e) {
    toast(String(e));
  }
}

let durationMode = null;

function untilFromChoice(choice) {
  if (choice === "never") return null;
  if (choice === "tomorrow") {
    const d = new Date();
    d.setHours(24, 0, 0, 0);
    return Math.floor(d.getTime() / 1000);
  }
  return Math.floor(Date.now() / 1000) + Number(choice) * 60;
}

function formatUntil(secs) {
  const d = new Date(secs * 1000);
  const sameDay = d.toDateString() === new Date().toDateString();
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  return sameDay ? time : `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
}

async function setStatusExpiry(changes) {
  const current = state.voice || {};
  const presenceUntil = "presence" in changes ? changes.presence : current.presence_until ?? null;
  const statusUntil = "status" in changes ? changes.status : current.status_until ?? null;
  try {
    state.voice = await invoke("set_status_expiry", { presenceUntil, statusUntil });
    if (state.user) state.user.presence_until = state.voice.presence_until ?? null;
  } catch (e) {
    toast(String(e));
  }
  renderProfilePopover();
  renderUserPanel();
}

function openDurations(mode, anchor) {
  durationMode = mode;
  $("pp-durations-title").textContent = mode.kind === "presence"
    ? `${({ invisible: "Appear Offline", ...PRESENCE_LABELS })[mode.presence] || "Status"} for how long?`
    : "Clear custom status after";
  $("pp-durations-never").textContent = mode.kind === "presence" ? "Until I turn it off" : "Don't clear";
  $("pp-durations-note").textContent = mode.kind === "presence" && mode.presence === "do_not_disturb"
    ? "Others see when you are back."
    : "";
  $("pp-durations-note").classList.toggle("hidden", !$("pp-durations-note").textContent);
  $("pp-custom-time").value = toLocalInput(new Date(Date.now() + 2 * 3600e3));
  const menu = $("pp-durations");
  menu.classList.remove("hidden");
  const pop = $("profile-pop").getBoundingClientRect();
  const a = anchor.getBoundingClientRect();
  const r = menu.getBoundingClientRect();
  const left = pop.right + 8 + r.width < window.innerWidth ? pop.right + 8 : Math.max(8, pop.left - r.width - 8);
  menu.style.left = `${left}px`;
  menu.style.top = `${Math.max(8, Math.min(a.top, window.innerHeight - r.height - 8))}px`;
}

function closeDurations() {
  $("pp-durations").classList.add("hidden");
  durationMode = null;
}

async function applyDuration(until) {
  const mode = durationMode;
  closeDurations();
  if (!mode) return;
  if (mode.kind === "presence") {
    await saveOwnProfile({ presence: mode.presence });
    await setStatusExpiry({ presence: until });
  } else {
    await setStatusExpiry({ status: until });
  }
}

function renderStatusExpiry() {
  const v = state.voice || {};
  const user = state.user || {};
  for (const btn of $("pp-presence").querySelectorAll("button")) {
    const small = btn.querySelector("small");
    if (!small) continue;
    if (!small.dataset.text) small.dataset.text = small.textContent;
    const active = btn.dataset.presence === (user.presence || "online");
    const until = active && btn.dataset.presence !== "online" ? v.presence_until : null;
    small.textContent = until ? `Until ${formatUntil(until)}` : small.dataset.text;
    small.classList.toggle("pp-until", Boolean(until));
  }
  const hasStatus = Boolean(user.custom_status || user.status_emoji);
  const expiry = $("pp-status-expiry");
  expiry.classList.toggle("hidden", !hasStatus);
  expiry.textContent = v.status_until ? `Clears at ${formatUntil(v.status_until)}` : "Clear after: Never";
}

const EMOJI_TOKEN_RE = /<:([a-z0-9_]{2,32}):([A-Za-z0-9]{1,128}@[A-Za-z0-9]{1,128})>/g;
const MAX_EMOJIS = 50;
const MAX_STICKERS = 30;
const MAX_EMOTICONS = 50;
const MAX_SOUNDS = 24;
const expressionUrls = new Map();

function expressionUrl(ref) {
  if (!expressionUrls.has(ref)) {
    expressionUrls.set(ref, invoke("fetch_expression", { blob: ref }).catch(() => {
      setTimeout(() => expressionUrls.delete(ref), 15000);
      return null;
    }));
  }
  return expressionUrls.get(ref);
}

function customEmojiImg(name, ref, cls = "custom-emoji") {
  const img = el("img", cls);
  img.alt = `:${name}:`;
  img.title = `:${name}:`;
  img.draggable = false;
  expressionUrl(ref).then(url => {
    if (url) img.src = url;
    else img.classList.add("missing");
  });
  return img;
}

function parseEmojiToken(text) {
  const m = /^<:([a-z0-9_]{2,32}):([A-Za-z0-9]{1,128}@[A-Za-z0-9]{1,128})>$/.exec(text || "");
  return m ? { name: m[1], image: m[2] } : null;
}

function decodeCustomEmojis(text) {
  const sticker = parseStickerToken(text);
  if (sticker) return `:${sticker.name}: sticker`;
  return String(text || "").replace(EMOJI_TOKEN_RE, ":$1:");
}

function currentRoles() {
  return (state.currentServer && state.currentServer.roles) || [];
}

function plainText(text) {
  return stripMarkdown(decodeRoleMentions(decodeCustomEmojis(text), currentRoles()).replace(/<:[^>\s]*$/, ""));
}

function encodeOutgoing(text) {
  const members = state.currentServer ? state.currentServer.members.map(m => m.name) : [];
  return encodeRoleMentions(encodeCustomEmojis(text), currentRoles(), members);
}

function isEmojiOnly(text) {
  const tokens = text.match(EMOJI_TOKEN_RE) || [];
  return tokens.length > 0 && tokens.length <= 27 && !text.replace(EMOJI_TOKEN_RE, "").trim();
}

const NO_SHARED_EXPRESSIONS = { emojis: [], stickers: [], emoticons: [] };
let shareable = NO_SHARED_EXPRESSIONS;

async function loadShareableEmojis() {
  try {
    shareable = { ...NO_SHARED_EXPRESSIONS, ...(await invoke("list_shareable_expressions")) };
  } catch {
    shareable = NO_SHARED_EXPRESSIONS;
  }
}

function serverHere() {
  return state.view === "server" ? state.currentServer : null;
}

function mayUseOtherServers() {
  const here = serverHere();
  return !here || here.can_use_external !== false;
}

function expressionGroups(kind) {
  const external = kind === "emoticons" || mayUseOtherServers();
  return pickerGroups(serverHere(), shareable[kind], kind, { external });
}

function usableCustomEmojis() {
  const groups = expressionGroups("emojis").map(g => ({ ...g, emojis: g.items }));
  return { local: groups.find(g => g.local) || null, external: groups.filter(g => !g.local) };
}

function findCustomEmoji(name) {
  const { local, external } = usableCustomEmojis();
  return (local && local.emojis.find(e => e.name === name))
    || external.flatMap(g => g.emojis).find(e => e.name === name)
    || catalogPickNamed(name)
    || null;
}

function encodeCustomEmojis(text) {
  return text.replace(/(?<![<\w]):([a-z0-9_]{2,32}):/gi, (whole, name) => {
    const e = findCustomEmoji(name.toLowerCase());
    return e ? `<:${e.name}:${e.image}>` : whole;
  });
}

function appendEmojiText(node, text) {
  const value = String(text || "");
  EMOJI_TOKEN_RE.lastIndex = 0;
  let last = 0;
  let m;
  while ((m = EMOJI_TOKEN_RE.exec(value))) {
    if (m.index > last) node.appendChild(document.createTextNode(value.slice(last, m.index)));
    node.appendChild(customEmojiImg(m[1], m[2], "custom-emoji profile-emoji"));
    last = m.index + m[0].length;
  }
  if (last < value.length) node.appendChild(document.createTextNode(value.slice(last)));
  return node;
}

function setEmojiText(node, text) {
  node.replaceChildren();
  return appendEmojiText(node, text);
}

function appendRichText(div, text) {
  EMOJI_TOKEN_RE.lastIndex = 0;
  let last = 0;
  let m;
  while ((m = EMOJI_TOKEN_RE.exec(text))) {
    if (m.index > last) appendTextWithMentions(div, text.slice(last, m.index));
    div.appendChild(customEmojiImg(m[1], m[2]));
    last = m.index + m[0].length;
  }
  if (last < text.length) appendTextWithMentions(div, text.slice(last));
}

function pickerSection(grid, title, { other = false, addKind = null } = {}) {
  const head = el("div", "emoji-section");
  head.appendChild(el("span", "emoji-section-name", title));
  if (other) head.appendChild(el("span", "emoji-section-tag", "Other Server"));
  if (addKind) {
    const add = el("button", "emoji-section-add");
    add.type = "button";
    add.title = `Add ${EXPRESSION_KINDS[addKind].one} to this server`;
    add.append(svgIcon("plus", 12), el("span", null, "Add"));
    add.onclick = e => {
      e.stopPropagation();
      openAddExpression(addKind);
    };
    head.appendChild(add);
  }
  grid.appendChild(head);
}

function canAddHere(kind) {
  const here = serverHere();
  return Boolean(here && here[`can_add_${kind}`]);
}

function appendExpressionGroups(grid, kind, query, renderItem, { localOnly = false, add = true } = {}) {
  const groups = expressionGroups(kind).filter(g => g.local || !localOnly);
  const label = kind === "emoticons" ? "text" : null;
  const addable = add && canAddHere(kind) && !query;
  if (addable && !groups.some(g => g.local)) {
    pickerSection(grid, serverHere().name, { addKind: kind });
    grid.appendChild(el("div", "picker-empty", `No ${EXPRESSION_KINDS[kind].many} here yet`));
  }
  let any = false;
  for (const group of groups) {
    const matches = expressionMatches(group.items, query, label);
    if (!matches.length) continue;
    any = true;
    pickerSection(grid, group.name, { other: !group.local, addKind: group.local && addable ? kind : null });
    for (const item of matches) grid.appendChild(renderItem(item));
  }
  return any;
}

const CATALOG_PICKER_SLOTS = 16;
const catalogPicker = { results: new Map(), timer: null, seq: 0 };
const catalogPicks = new Map();

function catalogPickNamed(name) {
  for (const pick of catalogPicks.values()) if (pick.name === name) return pick;
  return null;
}

const CATALOG_PICKER = {
  emojis: {
    label: "emoji.gg", slots: CATALOG_PICKER_SLOTS, cls: "catalog-slot", external: true,
    search: searchEmojiggPicks, use: (item, slot) => useCatalogEmoji(item, slot),
  },
  stickers: {
    label: "emoji.gg", slots: 6, cls: "sticker-tile catalog-sticker", external: true,
    search: searchEmojiggPicks, use: (item, slot) => useCatalogSticker(item, slot),
  },
  klipy: {
    label: "KLIPY", slots: 9, cls: "sticker-tile catalog-sticker", external: false,
    search: async q => {
      const page = await invoke("sticker_search", { query: q, from: "stickers", page: 1 });
      return page ? page.results : [];
    },
    use: (item, slot) => useCatalogSticker(item, slot),
  },
};

async function searchEmojiggPicks(q) {
  const page = await invoke("search_emoji_catalog", { query: q, category: null, page: 0 });
  return (page.results || []).slice(0, CATALOG_PICKER_SLOTS);
}

function appendCatalogSection(grid, q, kind) {
  const spec = CATALOG_PICKER[kind];
  if (spec.external && !mayUseOtherServers()) return false;
  if (spec.external && !account.server_emojis) {
    if (!q) return false;
    pickerSection(grid, "emoji.gg");
    grid.appendChild(el("div", "picker-empty", "Your account server doesn't offer emoji.gg search yet."));
    return true;
  }
  pickerSection(grid, spec.label);
  const slots = [];
  for (let i = 0; i < spec.slots; i++) {
    const slot = el("button", spec.cls);
    slot.type = "button";
    slot.disabled = true;
    grid.appendChild(slot);
    slots.push(slot);
  }
  const fill = all => {
    if (!slots[0].isConnected) return;
    const results = all.slice(0, spec.slots);
    if (!results.length) {
      const box = kind === "emojis" ? "emojis" : "stickers";
      slots[0].replaceWith(el("div", `picker-empty catalog-none ${box}`, `Nothing on ${spec.label} matches that`));
      for (const slot of slots.slice(1)) slot.remove();
      return;
    }
    slots.forEach((slot, i) => {
      const item = results[i];
      if (!item) return;
      slot.disabled = false;
      slot.title = `${item.title} from ${spec.label}`;
      const img = el("img");
      img.loading = "lazy";
      img.alt = item.title;
      img.draggable = false;
      img.src = viaAccountServer(item.preview || item.url);
      img.onerror = () => slot.classList.add("missing");
      slot.appendChild(img);
      slot.onclick = () => spec.use(item, slot);
    });
  };
  const key = `${spec.label}:${q}`;
  const cached = catalogPicker.results.get(key);
  if (cached) {
    fill(cached);
    return true;
  }
  const seq = ++catalogPicker.seq;
  clearTimeout(catalogPicker.timer);
  catalogPicker.timer = setTimeout(async () => {
    try {
      const results = await spec.search(q);
      catalogPicker.results.set(key, results);
      if (seq === catalogPicker.seq) fill(results);
    } catch {
      if (seq === catalogPicker.seq) fill([]);
    }
  }, q ? 300 : 0);
  return true;
}

const catalogStickers = new Map();

async function useCatalogSticker(item, slot) {
  if (slot.classList.contains("busy")) return;
  const known = catalogStickers.get(item.id);
  if (known) {
    sendSticker(known);
    return;
  }
  slot.classList.add("busy");
  try {
    const src = await invoke("fetch_catalog_image", { url: item.url });
    const prepared = await prepareEmojiImage(src, CATALOG_SIZES.stickers);
    const name = expressionName(item.title, "sticker");
    const token = parseStickerToken(await invoke("store_catalog_sticker", { name, data: prepared.data }));
    if (!token) throw new Error("that sticker could not be saved");
    catalogStickers.set(item.id, token);
    sendSticker(token);
  } catch (e) {
    toast(String(e.message || e));
  } finally {
    slot.classList.remove("busy");
  }
}

async function useCatalogEmoji(item, slot) {
  if (slot.classList.contains("busy")) return;
  const known = catalogPicks.get(item.id);
  if (known) {
    insertIntoComposer(`:${known.name}: `);
    return;
  }
  slot.classList.add("busy");
  try {
    const { local, external } = usableCustomEmojis();
    const taken = [local, ...external].filter(Boolean).flatMap(g => g.emojis.map(e => e.name))
      .concat([...catalogPicks.values()].map(p => p.name));
    const name = freeExpressionName(expressionName(item.title, "emoji"), taken);
    const src = await invoke("fetch_catalog_image", { url: item.url });
    const prepared = await prepareEmojiImage(src, CATALOG_SIZES.emojis);
    const token = parseEmojiToken(await invoke("store_catalog_emoji", { name, data: prepared.data }));
    if (!token) throw new Error("that emoji could not be saved");
    catalogPicks.set(item.id, token);
    insertIntoComposer(`:${token.name}: `);
  } catch (e) {
    toast(String(e.message || e));
  } finally {
    slot.classList.remove("busy");
  }
}

async function prepareEmojiImage(src, { size = 128, maxGif = 256 * 1024 } = {}) {
  const blob = await (await fetch(src)).blob();
  const EMOJI_SIZE_PX = size;
  if ((blob.type === "image/gif" || blob.type === "image/webp") && blob.size <= maxGif) {
    return { data: await blobToBase64(blob), url: src, flattened: false };
  }
  const img = await new Promise((resolve, reject) => {
    const i = new Image();
    i.onload = () => resolve(i);
    i.onerror = () => reject(new Error("could not read that image"));
    i.src = src;
  });
  const scale = Math.min(1, EMOJI_SIZE_PX / Math.max(img.naturalWidth, img.naturalHeight));
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
  const ctx = canvas.getContext("2d");
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
  const url = canvas.toDataURL("image/png");
  return { data: url.split(",")[1], url, flattened: blob.type === "image/gif" };
}

function blobToBase64(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",")[1]);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

function expressionName(raw, fallback) {
  const name = String(raw || "").toLowerCase().replace(/[^a-z0-9_]+/g, "_").replace(/^_+|_+$/g, "").slice(0, 32);
  return name.length >= 2 ? name : fallback;
}

function freeExpressionName(base, taken) {
  if (!taken.includes(base)) return base;
  for (let i = 2; ; i++) {
    const next = `${base.slice(0, 29)}_${i}`;
    if (!taken.includes(next)) return next;
  }
}

function pendingExpressionRow(preview, name, onSave) {
  const row = el("div", "expr-row pending");
  row.appendChild(preview);
  const input = el("input", "expr-name-input");
  input.type = "text";
  input.maxLength = 32;
  input.value = name;
  input.spellcheck = false;
  row.appendChild(input);
  const actions = el("div", "expr-actions");
  const save = el("button", "btn-primary btn-small", "Save");
  const cancel = el("button", "btn-link", "Cancel");
  actions.append(cancel, save);
  row.appendChild(actions);
  const submit = async () => {
    const clean = expressionName(input.value, "");
    if (!clean) {
      toast("Names need at least 2 letters, numbers or underscores");
      return;
    }
    save.disabled = true;
    try {
      await onSave(clean);
    } catch (e) {
      save.disabled = false;
      toast(String(e));
    }
  };
  save.onclick = submit;
  cancel.onclick = () => row.remove();
  input.addEventListener("keydown", e => {
    if (e.key === "Enter") submit();
    if (e.key === "Escape") {
      e.stopPropagation();
      row.remove();
    }
  });
  setTimeout(() => {
    input.focus();
    input.select();
  }, 0);
  return row;
}

function expressionDeleteButton(label, onDelete) {
  const del = el("button", "icon-btn expr-delete");
  del.title = label;
  del.appendChild(svgIcon("trash", 18));
  del.onclick = async () => {
    try {
      await onDelete();
      await refreshExpressions();
    } catch (err) {
      toast(String(err));
    }
  };
  return del;
}

function renderImageExpressions(server, { kind, listId, countId, max, removeCommand, previewClass, label }) {
  const items = server[kind] || [];
  $(countId).textContent = `${items.length} of ${max} slots used`;
  const list = $(listId);
  list.replaceChildren();
  for (const item of items) {
    const row = el("div", "expr-row");
    row.appendChild(customEmojiImg(item.name, item.image, previewClass));
    row.appendChild(el("span", "expr-name", `:${item.name}:`));
    if (server.can_manage_expressions) {
      row.appendChild(expressionDeleteButton(`Delete :${item.name}:`, () => invoke(removeCommand, { serverId: server.id, name: item.name })));
    }
    list.appendChild(row);
  }
  if (!items.length) list.appendChild(el("div", "manage-empty", `No ${label} yet`));
}

function renderEmojiSettings(server) {
  const emojis = server.emojis || [];
  $("emoji-share-toggle").checked = Boolean(server.emojis_shareable);
  $("emoji-share-toggle").disabled = !server.can_manage_server;
  $("emoji-share-row").classList.toggle("readonly", !server.can_manage_server);
  $("emoji-upload").classList.toggle("hidden", !server.can_add_emojis);
  $("emoji-upload").disabled = emojis.length >= MAX_EMOJIS;
  $("emoji-browse").classList.toggle("hidden", !server.can_add_emojis || !account.server_emojis);
  $("emoji-browse").disabled = emojis.length >= MAX_EMOJIS;
  renderRoleAccess(server, "emojis");
  renderImageExpressions(server, {
    kind: "emojis", listId: "emoji-list", countId: "emoji-count", max: MAX_EMOJIS,
    removeCommand: "remove_emoji", previewClass: "custom-emoji expr-preview", label: "custom emojis",
  });
}

function renderStickerSettings(server) {
  const stickers = server.stickers || [];
  $("sticker-share-toggle").checked = Boolean(server.stickers_shareable);
  $("sticker-share-toggle").disabled = !server.can_manage_server;
  $("sticker-share-row").classList.toggle("readonly", !server.can_manage_server);
  $("sticker-upload").classList.toggle("hidden", !server.can_add_stickers);
  $("sticker-upload").disabled = stickers.length >= MAX_STICKERS;
  $("sticker-browse").classList.toggle("hidden", !server.can_add_stickers || !syncCatalogBrowse());
  $("sticker-browse").disabled = stickers.length >= MAX_STICKERS;
  renderRoleAccess(server, "stickers");
  renderImageExpressions(server, {
    kind: "stickers", listId: "sticker-list", countId: "sticker-count", max: MAX_STICKERS,
    removeCommand: "remove_sticker", previewClass: "sticker-img expr-preview sticker-preview", label: "stickers",
  });
}

function renderEmoticonSettings(server) {
  const faces = server.emoticons || [];
  $("emoticon-share-toggle").checked = Boolean(server.emoticons_shareable);
  $("emoticon-share-toggle").disabled = !server.can_manage_server;
  $("emoticon-share-row").classList.toggle("readonly", !server.can_manage_server);
  $("emoticon-add").classList.toggle("hidden", !server.can_add_emoticons);
  $("emoticon-add").disabled = faces.length >= MAX_EMOTICONS;
  $("emoticon-count").textContent = `${faces.length} of ${MAX_EMOTICONS} slots used`;
  renderRoleAccess(server, "emoticons");
  const list = $("emoticon-list");
  list.replaceChildren();
  for (const face of faces) {
    const row = el("div", "expr-row");
    row.appendChild(el("span", "expr-face", face.text));
    row.appendChild(el("span", "expr-name", `:${face.name}:`));
    if (server.can_manage_expressions) {
      row.appendChild(expressionDeleteButton(`Delete :${face.name}:`, () => invoke("remove_emoticon", { serverId: server.id, name: face.name })));
    }
    list.appendChild(row);
  }
  if (!faces.length) list.appendChild(el("div", "manage-empty", "No emoticons yet"));
}

async function uploadEmoji() {
  const server = state.currentServer;
  if (!server) return;
  let src;
  try {
    src = await chooseImage("Choose an emoji image", $("server-image-file"));
  } catch (e) {
    toast(String(e));
    return;
  }
  if (!src) return;
  let prepared;
  try {
    prepared = await prepareEmojiImage(src);
  } catch (e) {
    toast(String(e.message || e));
    return;
  }
  if (prepared.flattened) toast("That GIF is over 256 KB, so it will be saved as a still image");
  const preview = el("img", "custom-emoji expr-preview");
  preview.src = prepared.url;
  const taken = (server.emojis || []).map(e => e.name);
  const row = pendingExpressionRow(preview, freeExpressionName("emoji", taken), async clean => {
    if (taken.includes(clean) && !confirm(`Replace the existing :${clean}: emoji?`)) return;
    await invoke("add_emoji", { serverId: server.id, name: clean, data: prepared.data });
    await refreshExpressions();
    toast(`Added :${clean}:`);
  });
  $("emoji-list").querySelector(".manage-empty")?.remove();
  $("emoji-list").prepend(row);
}

async function refreshExpressions() {
  await refreshCurrentServer();
  await loadShareableEmojis();
  if (!$("server-settings").classList.contains("hidden")) openServerSettings();
}

const SOUNDBOARD_KEY = "commy.soundboard";

function soundboardPrefs() {
  const defaults = { enabled: true, volume: 0.6 };
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(SOUNDBOARD_KEY) || "{}") };
  } catch {
    return defaults;
  }
}

function saveSoundboardPrefs(prefs) {
  try { localStorage.setItem(SOUNDBOARD_KEY, JSON.stringify(prefs)); } catch {}
}

const DEFAULT_SOUNDS = [
  { id: "airhorn", label: "Airhorn", file: "sounds/soundboard/airhorn.mp3" },
  { id: "ba_dum_tss", label: "Ba Dum Tss", file: "sounds/soundboard/ba-dum-tss.mp3" },
  { id: "cricket", label: "Crickets", file: "sounds/soundboard/cricket.mp3" },
  { id: "losing_horn", label: "Losing Horn", file: "sounds/soundboard/losing_horn.mp3" },
];

function defaultSound(name) {
  const id = String(name || "").startsWith("default:") ? name.slice(8) : null;
  return id ? DEFAULT_SOUNDS.find(s => s.id === id) || null : null;
}

const soundboardPlaying = new Set();

async function playExpressionSound(ref, volume) {
  const builtin = defaultSound(ref);
  const url = builtin ? builtin.file : await expressionUrl(ref);
  if (!url) return;
  const MAX_OVERLAPPING_CLIPS = 3;
  const STOP_A_CLIP_THAT_RUNS_ON_MS = (MAX_SOUND_SECONDS + 1) * 1000;
  if (soundboardPlaying.size >= MAX_OVERLAPPING_CLIPS) {
    const oldest = soundboardPlaying.values().next().value;
    oldest.pause();
    soundboardPlaying.delete(oldest);
  }
  const audio = new Audio(url);
  audio.volume = Math.max(0, Math.min(1, volume));
  soundboardPlaying.add(audio);
  const done = () => soundboardPlaying.delete(audio);
  audio.addEventListener("ended", done);
  setTimeout(() => {
    audio.pause();
    done();
  }, STOP_A_CLIP_THAT_RUNS_ON_MS);
  audio.play().catch(done);
}

function onSoundboardPlay(play) {
  const prefs = soundboardPrefs();
  flashSoundboardUser(play.user, defaultSound(play.name)?.label || play.name);
  if (!prefs.enabled || state.voice.deafened) return;
  playExpressionSound(play.blob, (play.volume ?? 1) * prefs.volume);
}

function flashSoundboardUser(userId, name) {
  for (const row of document.querySelectorAll(`.voice-user[data-user="${CSS.escape(userId)}"]`)) {
    row.querySelector(".sb-flash")?.remove();
    const tag = el("span", "sb-flash");
    tag.appendChild(svgIcon("soundboard", 12));
    tag.appendChild(el("span", null, name));
    row.appendChild(tag);
    setTimeout(() => tag.remove(), 2500);
  }
}

function pickSoundFile() {
  if (IS_TAURI) return invoke("pick_sound");
  return new Promise(resolve => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "audio/*,.mp3,.ogg,.wav,.m4a,.aac,.flac,.opus";
    input.onchange = () => {
      const file = input.files && input.files[0];
      if (!file) return resolve(null);
      if (file.size > SOUND_SOURCE_MAX_BYTES) {
        toast(`Pick an audio file under ${SOUND_SOURCE_MAX_BYTES / 1024 / 1024} MB`);
        return resolve(null);
      }
      const reader = new FileReader();
      reader.onload = () => resolve([file.name.replace(/\.[^.]+$/, ""), String(reader.result)]);
      reader.readAsDataURL(file);
    };
    input.click();
  });
}

function soundRow(server, sound, canDelete) {
  const row = el("div", "expr-row");
  const play = el("button", "icon-btn sound-preview");
  play.title = "Preview";
  play.appendChild(svgIcon("play", 18));
  play.onclick = () => playExpressionSound(sound.blob, (sound.volume ?? 1) * soundboardPrefs().volume);
  row.appendChild(play);
  if (canDelete || sound.emoji) {
    const icon = el("button", "icon-btn sound-emoji-btn");
    icon.type = "button";
    icon.replaceChildren(sound.emoji ? soundEmojiNode(sound.emoji) : svgIcon("smilePlus", 18));
    if (canDelete) {
      icon.title = "Choose an emoji for this sound";
      const setEmoji = async emoji => {
        try {
          await invoke("set_sound_emoji", { serverId: server.id, name: sound.name, emoji });
          await refreshExpressions();
        } catch (err) {
          toast(String(err));
        }
      };
      icon.onclick = e => openEmojiPop(e.currentTarget, setEmoji, { custom: true, onClear: sound.emoji ? () => setEmoji(null) : null });
    } else {
      icon.disabled = true;
    }
    row.appendChild(icon);
  }
  row.appendChild(el("span", "expr-name", sound.name));
  if (canDelete) {
    const del = el("button", "icon-btn expr-delete");
    del.title = `Delete ${sound.name}`;
    del.appendChild(svgIcon("trash", 18));
    del.onclick = async () => {
      try {
        await invoke("remove_sound", { serverId: server.id, name: sound.name });
        await refreshExpressions();
      } catch (err) {
        toast(String(err));
      }
    };
    row.appendChild(del);
  }
  return row;
}

function renderSoundboardSettings(server) {
  const sounds = server.sounds || [];
  $("sound-open-toggle").checked = Boolean(server.soundboard_open);
  $("sound-open-toggle").disabled = !server.can_manage_server;
  $("sound-open-row").classList.toggle("readonly", !server.can_manage_server);
  $("sound-share-toggle").checked = Boolean(server.sounds_shareable);
  $("sound-share-toggle").disabled = !server.can_manage_server;
  $("sound-share-row").classList.toggle("readonly", !server.can_manage_server);
  $("sound-upload").classList.toggle("hidden", !server.can_upload_sounds);
  $("sound-upload").disabled = sounds.length >= MAX_SOUNDS;
  $("sound-count").textContent = `${sounds.length} of ${MAX_SOUNDS} sounds`;
  const list = $("sound-list");
  list.replaceChildren();
  for (const sound of sounds) list.appendChild(soundRow(server, sound, server.can_manage_expressions));
  if (!sounds.length) list.appendChild(el("div", "manage-empty", "No sounds yet"));
  const builtins = $("sound-default-list");
  builtins.replaceChildren(...DEFAULT_SOUNDS.map(s => soundRow(server, { name: s.label, blob: `default:${s.id}`, volume: 1 }, false)));
}

const SOUND_WAVE_BARS = 96;
const soundDraft = { server: null, bytes: null, buffer: null, sel: null, emoji: null, afterSave: null, preview: null };

async function openSoundUpload(server, afterSave = null) {
  if (!server) return;
  let picked;
  try {
    picked = await pickSoundFile();
  } catch (e) {
    toast(String(e));
    return;
  }
  if (!picked) return;
  const [stem, dataUrl] = picked;
  let bytes;
  let buffer;
  try {
    bytes = new Uint8Array(await (await fetch(dataUrl)).arrayBuffer());
    buffer = await decodeSoundSource(bytes);
  } catch {
    toast("Could not read that sound file");
    return;
  }
  if (buffer.duration > SOUND_SOURCE_MAX_SECONDS) {
    toast(`Pick a clip under ${SOUND_SOURCE_MAX_SECONDS / 60} minutes`);
    return;
  }
  stopSoundPreview();
  $("soundboard-pop").classList.add("hidden");
  Object.assign(soundDraft, { server, bytes, buffer, sel: initialSoundSelection(buffer.duration, MAX_SOUND_SECONDS), emoji: null, afterSave });
  const peaks = soundWaveformPeaks(Array.from({ length: buffer.numberOfChannels }, (_, i) => buffer.getChannelData(i)), SOUND_WAVE_BARS);
  $("sound-wave").replaceChildren(...peaks.map(p => {
    const bar = el("span");
    bar.style.height = `${Math.max(4, Math.round(p * 100))}%`;
    return bar;
  }));
  $("sound-hint").textContent = buffer.duration > MAX_SOUND_SECONDS + 0.2
    ? `This clip is ${formatTrimTime(buffer.duration)} long. Drag the handles to pick up to ${MAX_SOUND_SECONDS} seconds.`
    : "Drag the handles to trim the start or end, then give it a name and an emoji.";
  $("sound-name").value = freeExpressionName(expressionName(stem, "sound"), (server.sounds || []).map(s => s.name));
  $("sound-save").disabled = false;
  paintSoundEmoji();
  paintSoundSelection();
  paintSoundPreviewButton();
  showModal("modal-sound");
  $("sound-name").focus();
  $("sound-name").select();
}

function paintSoundEmoji() {
  $("sound-emoji").replaceChildren(soundDraft.emoji ? soundEmojiNode(soundDraft.emoji) : svgIcon("smilePlus", 20));
}

function paintSoundSelection() {
  const { buffer, sel } = soundDraft;
  if (!buffer) return;
  const pct = t => `${(t / buffer.duration) * 100}%`;
  $("sound-shade-start").style.width = pct(sel.start);
  $("sound-shade-end").style.width = pct(buffer.duration - sel.end);
  $("sound-window").style.left = pct(sel.start);
  $("sound-window").style.width = pct(sel.end - sel.start);
  $("sound-handle-start").style.left = pct(sel.start);
  $("sound-handle-end").style.left = pct(sel.end);
  const bars = $("sound-wave").children;
  for (let i = 0; i < bars.length; i++) {
    const mid = ((i + 0.5) / bars.length) * buffer.duration;
    bars[i].classList.toggle("on", mid >= sel.start && mid <= sel.end);
  }
  $("sound-trim-times").textContent = `${formatTrimTime(sel.start)} to ${formatTrimTime(sel.end)}, ${(sel.end - sel.start).toFixed(1)} s`;
}

function moveSoundDraft(which, at) {
  stopSoundPreview();
  soundDraft.sel = moveSoundSelection(soundDraft.sel, which, at, soundDraft.buffer.duration, MAX_SOUND_SECONDS);
  paintSoundSelection();
}

function soundTrimTimeAt(clientX) {
  const rect = $("sound-trim").getBoundingClientRect();
  const frac = rect.width ? (clientX - rect.left) / rect.width : 0;
  return Math.max(0, Math.min(1, frac)) * soundDraft.buffer.duration;
}

function startSoundTrimDrag(e) {
  if (!soundDraft.buffer || e.button > 0) return;
  e.preventDefault();
  const at = soundTrimTimeAt(e.clientX);
  const { sel } = soundDraft;
  let which;
  let offset = 0;
  if (e.target.id === "sound-handle-start") which = "start";
  else if (e.target.id === "sound-handle-end") which = "end";
  else if (e.target.id === "sound-window") {
    which = "both";
    offset = at - sel.start;
  } else {
    which = Math.abs(at - sel.start) <= Math.abs(at - sel.end) ? "start" : "end";
    moveSoundDraft(which, at);
  }
  if (which !== "both") $(`sound-handle-${which}`).focus();
  const track = $("sound-trim");
  track.setPointerCapture(e.pointerId);
  const move = ev => moveSoundDraft(which, soundTrimTimeAt(ev.clientX) - offset);
  const up = () => {
    track.removeEventListener("pointermove", move);
    track.removeEventListener("pointerup", up);
    track.removeEventListener("pointercancel", up);
  };
  track.addEventListener("pointermove", move);
  track.addEventListener("pointerup", up);
  track.addEventListener("pointercancel", up);
}

function nudgeSoundHandle(which, e) {
  const step = { ArrowLeft: -0.1, ArrowRight: 0.1, ArrowDown: -0.1, ArrowUp: 0.1 }[e.key];
  if (!step || !soundDraft.buffer) return;
  e.preventDefault();
  moveSoundDraft(which, soundDraft.sel[which] + step * (e.shiftKey ? 10 : 1));
}

function paintSoundPreviewButton() {
  const playing = Boolean(soundDraft.preview);
  $("sound-trim-play").replaceChildren(svgIcon(playing ? "pause" : "play", 16));
  $("sound-trim-play").title = playing ? "Stop" : "Play the selected part";
  $("sound-playhead").classList.toggle("hidden", !playing);
}

function stopSoundPreview() {
  const preview = soundDraft.preview;
  if (!preview) return;
  soundDraft.preview = null;
  cancelAnimationFrame(preview.frame);
  try { preview.source.stop(); } catch {}
  preview.ctx.close().catch(() => {});
  paintSoundPreviewButton();
}

function toggleSoundPreview() {
  if (soundDraft.preview) return stopSoundPreview();
  const { buffer, sel } = soundDraft;
  if (!buffer) return;
  const ctx = new AudioContext();
  const source = ctx.createBufferSource();
  source.buffer = buffer;
  const gain = ctx.createGain();
  gain.gain.value = soundboardPrefs().volume;
  source.connect(gain).connect(ctx.destination);
  const began = ctx.currentTime;
  source.start(0, sel.start, sel.end - sel.start);
  const preview = { ctx, source, frame: 0 };
  source.onended = () => { if (soundDraft.preview === preview) stopSoundPreview(); };
  const tick = () => {
    const at = sel.start + (ctx.currentTime - began);
    $("sound-playhead").style.left = `${(Math.min(at, sel.end) / buffer.duration) * 100}%`;
    preview.frame = requestAnimationFrame(tick);
  };
  soundDraft.preview = preview;
  paintSoundPreviewButton();
  tick();
}

async function saveSoundDraft() {
  const { server, emoji, afterSave } = soundDraft;
  if (!server || !soundDraft.buffer) return;
  const name = expressionName($("sound-name").value, "");
  if (!name) {
    toast("Names are 2 to 32 letters, numbers or underscores");
    return;
  }
  const exists = (server.sounds || []).some(s => s.name === name);
  if (exists && !server.can_manage_expressions) {
    toast(`${name} is taken, pick another name`);
    return;
  }
  if (exists && !confirm(`Replace the existing ${name}?`)) return;
  stopSoundPreview();
  $("sound-save").disabled = true;
  try {
    const bytes = await renderTrimmedSound(soundDraft.bytes, soundDraft.buffer, soundDraft.sel);
    if (bytes.length > SOUND_STORE_MAX_BYTES) throw new Error("That clip is still over 1 MB, pick a shorter part");
    await invoke("add_sound", { serverId: server.id, name, data: await blobToBase64(new Blob([bytes])), volume: 1, emoji });
    hideModals();
    await refreshExpressions();
    toast(`Added ${name} to the soundboard`);
    if (afterSave) afterSave();
  } catch (e) {
    toast(String(e.message || e));
  } finally {
    $("sound-save").disabled = false;
  }
}

function wireSoundUpload() {
  $("sound-trim").addEventListener("pointerdown", startSoundTrimDrag);
  $("sound-handle-start").addEventListener("keydown", e => nudgeSoundHandle("start", e));
  $("sound-handle-end").addEventListener("keydown", e => nudgeSoundHandle("end", e));
  $("sound-trim-play").onclick = toggleSoundPreview;
  $("sound-emoji").onclick = e => {
    const set = emoji => {
      soundDraft.emoji = emoji;
      paintSoundEmoji();
    };
    openEmojiPop(e.currentTarget, set, { custom: true, onClear: soundDraft.emoji ? () => set(null) : null });
  };
  $("sound-name").addEventListener("input", e => {
    const clean = soundNameDraft(e.target.value);
    if (clean !== e.target.value) e.target.value = clean;
  });
  $("sound-name").addEventListener("keydown", e => {
    if (e.key === "Enter") saveSoundDraft();
  });
  $("sound-save").onclick = saveSoundDraft;
  $("sound-cancel").onclick = hideModals;
}

let soundboardCooldown = 0;

async function toggleSoundboardPop() {
  const pop = $("soundboard-pop");
  if (!pop.classList.contains("hidden")) {
    pop.classList.add("hidden");
    return;
  }
  const serverId = state.voiceServer && state.voiceServer.id;
  if (!serverId) return;
  closeFloating("soundboard-pop");
  let server = state.currentServer && state.currentServer.id === serverId ? state.currentServer : null;
  try {
    server = server || await invoke("get_server", { serverId });
  } catch (e) {
    toast(String(e));
    return;
  }
  const grid = $("sb-grid");
  grid.replaceChildren();
  const canAdd = server.can_upload_sounds && (server.sounds || []).length < MAX_SOUNDS;
  $("sb-add").classList.toggle("hidden", !canAdd);
  $("sb-add").onclick = () => openSoundUpload(server, toggleSoundboardPop);
  const addButton = (name, label, emoji) => {
    const btn = el("button", "sb-sound");
    btn.appendChild(emoji ? soundEmojiNode(emoji) : svgIcon("soundboard", 14));
    btn.appendChild(el("span", null, label));
    btn.title = `Play ${label} for everyone in the channel`;
    btn.disabled = !server.can_use_soundboard;
    btn.onclick = async () => {
      const now = Date.now();
      if (now < soundboardCooldown) return;
      soundboardCooldown = now + 1500;
      try {
        await invoke("play_soundboard", { serverId: server.id, name });
      } catch (e) {
        toast(String(e));
      }
    };
    grid.appendChild(btn);
  };
  grid.appendChild(el("div", "sb-section", server.name));
  for (const sound of server.sounds || []) addButton(sound.name, sound.name, sound.emoji);
  if (!(server.sounds || []).length) {
    grid.appendChild(el("div", "sb-empty", server.can_upload_sounds
      ? "No server sounds yet. Choose Add sound to upload one."
      : "This server has no sounds of its own yet."));
  }
  let others = [];
  try {
    others = (await invoke("list_shareable_sounds")).filter(s => s.server !== server.id);
  } catch {}
  let lastServer = null;
  for (const sound of others) {
    if (sound.server !== lastServer) {
      grid.appendChild(el("div", "sb-section", sound.server_name));
      lastServer = sound.server;
    }
    const btn = el("button", "sb-sound");
    btn.appendChild(sound.emoji ? soundEmojiNode(sound.emoji) : svgIcon("soundboard", 14));
    btn.appendChild(el("span", null, sound.name));
    btn.title = `Play ${sound.name} from ${sound.server_name} for everyone in the channel`;
    btn.disabled = !server.can_use_soundboard;
    btn.onclick = async () => {
      const now = Date.now();
      if (now < soundboardCooldown) return;
      soundboardCooldown = now + 1500;
      try {
        await invoke("play_shared_sound", { serverId: server.id, sourceServerId: sound.server, name: sound.name });
      } catch (e) {
        toast(String(e));
      }
    };
    grid.appendChild(btn);
  }
  grid.appendChild(el("div", "sb-section", "Default"));
  for (const sound of DEFAULT_SOUNDS) addButton(`default:${sound.id}`, sound.label);
  $("sb-muted").classList.toggle("hidden", soundboardPrefs().enabled);
  pop.classList.remove("hidden");
  placePopup(pop, $("vp-soundboard"), "above");
}

const EVERYONE_CANDIDATES = [
  { id: "everyone", name: "everyone", special: "Notify everyone in this server", rank: 2 },
  { id: "here", name: "here", special: "Notify everyone online", rank: 2 },
];

function mentionPeople() {
  if (state.view === "dm" && state.currentDm) {
    const me = state.user ? [{ id: state.user.id, name: state.user.display_name || state.user.username }] : [];
    return [{ id: state.currentDm.user, name: state.currentDm.name }, ...me];
  }
  if (!state.currentServer) return [];
  return state.currentServer.members.map(m => ({ id: m.id, name: m.name, presence: m.presence, rank: 0 }));
}

function mentionNames() {
  const people = mentionPeople().map(c => c.name);
  return state.view === "dm" ? people : [...people, ...EVERYONE_MENTIONS];
}

function mentionCandidates() {
  const people = mentionPeople();
  if (state.view === "dm" || !state.currentServer || !state.user) return people;
  const server = state.currentServer;
  const roles = mentionableRoles(server, state.user.id)
    .map(r => ({ id: r.id, name: r.name, color: r.color, role: true, special: `Notify members with ${r.name}`, rank: 1 }));
  const everyone = memberCanMentionEveryone(server, state.user.id) ? EVERYONE_CANDIDATES : [];
  return [...people, ...roles, ...everyone];
}

let mentionRegexCache = { key: "", re: null };

function mentionRegex() {
  const names = mentionNames().filter(Boolean);
  const key = names.join("\u0000");
  if (mentionRegexCache.key !== key) {
    const escaped = [...new Set(names)]
      .sort((a, b) => b.length - a.length)
      .map(n => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
    mentionRegexCache = {
      key,
      re: escaped.length ? new RegExp(`@(${escaped.join("|")})(?![\\p{L}\\p{N}_])`, "giu") : null,
    };
  }
  return mentionRegexCache.re;
}

const renderingMessage = { author: null };

function senderMayMentionEveryone() {
  return state.view === "server" && memberCanMentionEveryone(state.currentServer, renderingMessage.author);
}

function roleMentionPill(role, forMe) {
  const pill = el("span", "mention role" + (forMe ? " me" : ""), `@${role.name}`);
  if (role.color) {
    pill.style.setProperty("--role-color", `#${role.color}`);
    pill.style.setProperty("--role-tint", roleTint(role.color));
  }
  return pill;
}

function roleTint(hex) {
  const n = parseInt(hex, 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, .18)`;
}

function appendRoleMention(div, roleId) {
  const server = state.view === "server" ? state.currentServer : null;
  const role = roleById(server, roleId);
  if (!role || !roleMentionAllowed(server, renderingMessage.author, role)) {
    div.appendChild(document.createTextNode(role ? `@${role.name}` : UNKNOWN_ROLE_MENTION));
    return;
  }
  const mine = state.user ? (serverMember(server, state.user.id) || {}).roles || [] : [];
  div.appendChild(roleMentionPill(role, mine.includes(role.id)));
}

function appendTextWithMentions(div, text) {
  for (const part of splitRoleMentions(text)) {
    if (part.role !== undefined) appendRoleMention(div, part.role);
    else appendNameMentions(div, part.text);
  }
}

function appendNameMentions(div, text) {
  const re = mentionRegex();
  if (!re) {
    div.appendChild(document.createTextNode(text));
    return;
  }
  const myNames = myMentionNames().filter(Boolean).map(n => n.toLowerCase());
  const everyoneAllowed = senderMayMentionEveryone();
  re.lastIndex = 0;
  let last = 0;
  let m;
  while ((m = re.exec(text))) {
    const name = m[1].toLowerCase();
    const everyone = isEveryoneMention(name);
    if (everyone && !everyoneAllowed) continue;
    if (m.index > last) div.appendChild(document.createTextNode(text.slice(last, m.index)));
    const forMe = myNames.includes(name) || everyone;
    div.appendChild(el("span", "mention" + (forMe ? " me" : ""), m[0]));
    last = m.index + m[0].length;
  }
  if (last < text.length) div.appendChild(document.createTextNode(text.slice(last)));
}

const suggestPop = { input: null, token: null, items: [], index: 0, commandInputs: new Set() };

function mentionSuggestions(query) {
  return mentionCandidates()
    .filter(c => c.name && c.name.toLowerCase().includes(query))
    .sort((a, b) => {
      const ap = a.name.toLowerCase().startsWith(query) ? 0 : 1;
      const bp = b.name.toLowerCase().startsWith(query) ? 0 : 1;
      return ap - bp || (a.rank || 0) - (b.rank || 0) || a.name.localeCompare(b.name);
    })
    .slice(0, SUGGEST_LIMIT)
    .map(c => ({ ...c, kind: "mention" }));
}

function emojiGroupsHere() {
  const { local, external } = usableCustomEmojis();
  const faces = expressionGroups("emoticons").map(g => ({ name: g.name, emojis: [], emoticons: g.items }));
  return [local, ...external].filter(Boolean).concat(faces, [{ name: "Commy", emojis: [], emoticons: builtinEmoticons() }]);
}

function tonedSuggestions(items) {
  const tone = emojiTone();
  return items.map(item => (item.kind === "standard" ? { ...item, emoji: withSkinTone(item.emoji, item.skin, tone, EMOJI_TONES) } : item));
}

function suggestionsFor(token, input) {
  if (token.kind === "mention") return mentionSuggestions(token.query);
  if (token.kind === "emoji") return tonedSuggestions(emojiSuggestions(token.query, emojiGroupsHere(), usableEmojiRows()));
  if (token.kind === "command" && suggestPop.commandInputs.has(input)) {
    const available = { gifs: Boolean(gifSettings().key) || serverHasGifs(), tts: state.view === "server" && canSendTtsHere() };
    return commandSuggestions(token, available, slashCatalogHere());
  }
  return [];
}

function slashCatalogHere() {
  return state.view === "server" && state.currentServer ? slashCatalog(state.currentServer) : [];
}

function slashUsageNode(item) {
  const span = el("span", "suggest-args suggest-usage");
  const at = item.args === null ? -1 : slashOptionAt(item.entry, item.args);
  item.entry.options.forEach((option, i) => {
    const word = el("span", "suggest-opt" + (i === at ? " on" : ""), option.required ? `<${option.name}>` : `[${option.name}]`);
    span.appendChild(word);
  });
  return span;
}

function suggestHeading(token) {
  if (token.kind === "command") return "Commands";
  if (token.kind === "emoji") return `Emoji matching :${token.query}`;
  const what = state.view === "dm" ? "Members" : "Members and roles";
  return token.query ? `${what} matching @${token.query}` : what;
}

function commandHint(item) {
  if (item.name === TTS_COMMAND) return ttsCommandHint(item);
  if (item.bot) return item.args === null ? item.entry.description : slashHint(item.entry, item.args).text;
  if (!item.available) return "GIF search is not set up";
  return item.args ? "Search GIFs" : "Search for a GIF";
}

function suggestionRow(item) {
  const row = el("button", "mention-row");
  if (item.kind === "custom") {
    const glyph = el("span", "suggest-glyph");
    glyph.appendChild(customEmojiImg(item.name, item.image));
    row.append(glyph, el("span", "mention-name", `:${item.name}:`), el("span", "mention-hint", item.server));
  } else if (item.kind === "emoticon") {
    row.append(el("span", "mention-name", `:${item.name}`), el("span", "suggest-face", item.text), el("span", "mention-hint", item.server));
  } else if (item.kind === "standard") {
    row.append(el("span", "suggest-glyph", item.emoji), el("span", "mention-name", `:${item.code}:`));
  } else if (item.kind === "command" && item.bot) {
    row.classList.add("slash-row");
    row.append(el("span", "mention-at", "/"), el("span", "mention-name", `/${item.name}`));
    if (item.entry.options.length) row.appendChild(slashUsageNode(item));
    row.appendChild(el("span", "mention-hint", commandHint(item)));
    row.title = `${item.entry.bot_name}: ${item.entry.description}`;
  } else if (item.kind === "command") {
    row.append(el("span", "mention-at", "/"), el("span", "mention-name", `/${item.name}`));
    if (item.args) row.appendChild(el("span", "suggest-args", item.args));
    row.appendChild(el("span", "mention-hint", commandHint(item)));
  } else if (item.role) {
    row.append(el("span", "mention-at", "@"), roleMentionPill(item, false), el("span", "mention-hint", item.special));
  } else if (item.special) {
    row.append(el("span", "mention-at", "@"), el("span", "mention-name", `@${item.name}`), el("span", "mention-hint", item.special));
  } else {
    row.append(makeAvatar(item.id, item.name, "avatar avatar-sm"), el("span", "mention-name", item.name));
  }
  return row;
}

function updateSuggestPop(input) {
  const names = suggestPop.commandInputs.has(input) ? slashNames(slashCatalogHere()) : [];
  const token = suggestTokenAt(input.value, input.selectionStart, names);
  const items = token ? suggestionsFor(token, input) : [];
  if (!items.length) {
    closeSuggestPop();
    return;
  }
  const pop = $("mention-pop");
  const sameList = !pop.classList.contains("hidden") && suggestPop.input === input && suggestPop.token && suggestPop.token.kind === token.kind;
  Object.assign(suggestPop, { input, token, items, index: sameList ? Math.min(suggestPop.index, items.length - 1) : 0 });
  pop.replaceChildren(el("div", "mention-head", suggestHeading(token)));
  items.forEach((item, i) => {
    const row = suggestionRow(item);
    row.classList.toggle("active", i === suggestPop.index);
    row.onmousedown = e => {
      e.preventDefault();
      pickSuggestion(i);
    };
    pop.appendChild(row);
  });
  const r = input.closest(".composer").getBoundingClientRect();
  pop.style.left = `${r.left}px`;
  pop.style.width = `${Math.min(r.width, 420)}px`;
  pop.style.bottom = `${window.innerHeight - r.top + 6}px`;
  pop.classList.remove("hidden");
  pop.querySelector(".mention-row.active")?.scrollIntoView({ block: "nearest" });
}

function closeSuggestPop() {
  $("mention-pop").classList.add("hidden");
  suggestPop.items = [];
  suggestPop.token = null;
}

function runComposerCommand(input, item) {
  input.value = "";
  closeSuggestPop();
  if (item.name === "gif") openGifPicker(item.args);
}

function pickSuggestion(i) {
  const item = suggestPop.items[i];
  const input = suggestPop.input;
  if (!item || !input) return;
  if (item.kind === "command" && item.ready && !item.bot) {
    runComposerCommand(input, item);
    return;
  }
  if (item.kind === "command" && item.bot && item.args !== null) {
    closeSuggestPop();
    return;
  }
  const caret = input.selectionStart ?? input.value.length;
  const insert = suggestionInsert(item);
  if (item.kind === "standard") rememberEmoji(item.emoji);
  if (item.kind === "custom") rememberEmoji(`<:${item.name}:${item.image}>`);
  const start = suggestPop.token.start;
  input.value = input.value.slice(0, start) + insert + input.value.slice(caret);
  const pos = start + insert.length;
  input.setSelectionRange(pos, pos);
  input.focus();
  updateSuggestPop(input);
}

function mentionTypedInFull() {
  const { token, items, index } = suggestPop;
  const item = items[index];
  return !!(token && token.kind === "mention" && item && item.name.toLowerCase() === token.query);
}

function slashTypedInFull() {
  const item = suggestPop.items[suggestPop.index];
  return !!(item && item.kind === "command" && item.bot && item.args !== null);
}

function wireSuggestions(input, { commands = false } = {}) {
  if (commands) suggestPop.commandInputs.add(input);
  input.addEventListener("input", () => updateSuggestPop(input));
  input.addEventListener("click", () => updateSuggestPop(input));
  input.addEventListener("blur", () => setTimeout(closeSuggestPop, 100));
  input.addEventListener("keydown", e => {
    if ($("mention-pop").classList.contains("hidden") || suggestPop.input !== input) return;
    const n = suggestPop.items.length;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      suggestPop.index = (suggestPop.index + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      updateSuggestPop(input);
    } else if (e.key === "Enter" && (mentionTypedInFull() || slashTypedInFull())) {
      closeSuggestPop();
      return;
    } else if (e.key === "Enter" || e.key === "Tab") {
      pickSuggestion(suggestPop.index);
    } else if (e.key === "Escape") {
      closeSuggestPop();
    } else {
      return;
    }
    e.preventDefault();
    e.stopImmediatePropagation();
  }, { capture: true });
}

function appendAfterText(content, mark) {
  const last = content.lastElementChild;
  if (last && last.classList.contains("md-para")) last.appendChild(mark);
  else content.appendChild(mark);
}

function buildMessageContent(text, authorId = null) {
  const div = el("div", "msg-content");
  const sticker = parseStickerToken(text);
  if (sticker) {
    div.classList.add("sticker-msg");
    div.appendChild(customEmojiImg(sticker.name, sticker.image, "sticker-img"));
    return div;
  }
  if (looksLikeInviteLink(text)) {
    div.appendChild(inviteCard(text));
    return div;
  }
  if (isBareMessageLink(text)) {
    div.appendChild(jumpCard(parseMessageLink(text)));
    return div;
  }
  renderingMessage.author = authorId;
  renderBlocks(div, parseMessage(text));
  renderingMessage.author = null;
  if (isEmojiOnly(text)) div.classList.add("jumbo");
  const invite = inviteCard(text);
  if (invite) div.appendChild(invite);
  const linked = parseMessageLink(text);
  if (linked) div.appendChild(jumpCard(linked));
  return div;
}

function renderCommandUse(message) {
  const used = message.command;
  const line = el("div", "cmd-use");
  line.append(el("span", "cmd-use-badge", "/"), el("span", "cmd-use-label", "used"), el("span", "cmd-use-name", `/${used.name}`));
  const entry = slashCatalogHere().find(c => c.name === used.name && c.bot === used.bot) || null;
  const channelName = id => ((state.currentServer && state.currentServer.channels) || []).find(c => c.id === id)?.name || null;
  for (const chip of slashChips(used, entry, { memberName: id => avatarNameFor(id), channelName })) {
    const pill = el("span", "cmd-use-chip");
    pill.append(el("span", "cmd-use-chip-name", chip.name), el("span", "cmd-use-chip-value", chip.value));
    pill.title = `${chip.name}: ${chip.value}`;
    line.appendChild(pill);
  }
  line.appendChild(el("span", "cmd-use-bot", `to ${used.bot_name || "a bot"}`));
  line.title = message.content || `/${used.name}`;
  return line;
}

function jumpCard(target) {
  const card = el("div", "jump-card");
  card.appendChild(svgIcon("link", 18));
  const known = state.servers.find(s => s.id === target.server);
  const here = state.currentServer && state.currentServer.id === target.server ? state.currentServer.channels.find(c => c.id === target.channel) : null;
  const gated = sensitiveGated({ view: "server", channel: here, confirmed: sensitiveConfirmed() });
  card.appendChild(el("span", "jump-card-text", gated ? `${SENSITIVE_PREVIEW} in ${known.name}` : known ? `Message in ${known.name}` : "Message in a server you are not in"));
  const go = el("button", "btn-primary btn-small", "Jump");
  go.disabled = !known;
  go.onclick = () => openMessageLink(target);
  card.appendChild(go);
  return card;
}

async function openMessageLink(target) {
  if (!state.currentServer || state.currentServer.id !== target.server) {
    if (!state.servers.some(s => s.id === target.server)) {
      toast("That message is in a server you are not in");
      return;
    }
    await selectServer(target.server);
  }
  const channel = state.currentServer && state.currentServer.channels.find(c => c.id === target.channel);
  if (!channel) {
    toast("That channel is gone");
    return;
  }
  if (!state.currentChannel || state.currentChannel.id !== channel.id) await selectChannel(channel);
  await jumpToMessage(target.message);
}

function copyText(text, done) {
  navigator.clipboard.writeText(text).then(() => toast(done), () => toast("Could not copy"));
}

function openMessageMenu(message, anchor) {
  closeFloating();
  const menu = $("msg-menu");
  menu.replaceChildren();
  const { item } = contextMenuBuilder(menu);
  if (message.content) item("Copy Text", () => copyText(plainText(message.content), "Text copied"));
  if (state.view === "server" && state.currentChannel && !state.currentServer.group) {
    item("Copy Message Link", () => copyText(messageLink({ server: state.currentServer.id, channel: state.currentChannel.id, message: message.id }), "Message link copied"));
  }
  if (message.content) item("Forward", () => openForward(message));
  if (message.content && speechAvailable()) item("Read Aloud", () => speakMessage(message));
  savedMenuItems(item, message);
  if (state.view === "server" && state.currentChannel && !message.pending) item("Mark as Unread", () => markUnreadFrom(message));
  const r = anchor.getBoundingClientRect();
  placeMenuAt(menu, r.left, r.bottom + 4);
}

const savedState = { saved: [], reminders: [], open: false, timer: null };

function isSavedMessage(id) {
  return savedState.saved.some(s => s.message === id);
}

function applySavedStore(store) {
  savedState.saved = sortSaved(store && store.saved);
  savedState.reminders = sortReminders(store && store.reminders);
  syncSavedButton();
  if (savedState.open) renderSavedView();
}

async function loadSaved() {
  try {
    applySavedStore(await invoke("saved_list"));
  } catch {
    applySavedStore(null);
  }
  await checkReminders();
  if (!savedState.timer) savedState.timer = setInterval(checkReminders, REMINDER_CHECK_MS);
}

function savedContext() {
  if (inDmView()) return { dm: state.currentDm };
  const channel = state.currentChannel ? { id: state.currentChannel.id, name: channelPlainName(state.currentChannel) } : null;
  return { server: state.currentServer, channel };
}

async function toggleSaved(message) {
  const saved = !isSavedMessage(message.id);
  try {
    applySavedStore(await invoke("saved_set", { entry: messageEntry(message, savedContext()), saved }));
    toast(saved ? "Message saved" : "Removed from Saved");
    renderMessages(state.renderedMessages || []);
  } catch (e) {
    toast(String(e));
  }
}

function savedMessageActions(add, message) {
  if (message.pending || message.held) return;
  const saved = isSavedMessage(message.id);
  add(saved ? "bookmarkOn" : "bookmark", saved ? "Unsave" : "Save", () => toggleSaved(message));
  add("bell", "Remind Me", e => openRemindPop(message, e.currentTarget));
}

function savedMenuItems(item, message) {
  if (message.pending || message.held) return;
  item(isSavedMessage(message.id) ? "Remove from Saved" : "Save Message", () => toggleSaved(message));
  item("Remind Me", () => openRemindPop(message, messageRow(message.id) || $("messages")));
}

function openRemindPop(message, anchor) {
  closeFloating();
  const pop = $("remind-pop");
  pop.replaceChildren(el("div", "ctx-title", "Remind me about this"));
  const choose = (label, onPick) => {
    const btn = el("button", null, label);
    btn.onclick = () => {
      closeRemindPop();
      onPick();
    };
    pop.appendChild(btn);
  };
  for (const preset of REMINDER_PRESETS) {
    if (preset.id === "custom") continue;
    choose(preset.label, () => setReminder(message, reminderDue(preset.id, Date.now())));
  }
  pop.appendChild(el("div", "ctx-sep"));
  const pick = el("div", "remind-pick");
  pick.appendChild(el("span", "remind-pick-label", "Pick a time"));
  const input = el("input");
  input.type = "datetime-local";
  input.min = toDateTimeLocal(Date.now() + 60e3);
  input.value = toDateTimeLocal(Date.now() + 3600e3);
  input.setAttribute("aria-label", "Reminder time");
  pick.appendChild(input);
  const set = el("button", "btn-primary btn-small", "Set");
  set.onclick = () => {
    const due = parseReminderTime(input.value, Date.now());
    if (!due) {
      toast("Pick a time that is still ahead");
      return;
    }
    closeRemindPop();
    setReminder(message, due);
  };
  pick.appendChild(set);
  pop.appendChild(pick);
  const r = anchor.getBoundingClientRect();
  placeMenuAt(pop, r.left, r.bottom + 4);
}

function closeRemindPop() {
  $("remind-pop").classList.add("hidden");
}

async function setReminder(message, due) {
  if (!due) return;
  try {
    applySavedStore(await invoke("reminder_set", { entry: messageEntry(message, savedContext()), due }));
    toast(`Reminder set · ${reminderLabel(due, Date.now())}`);
  } catch (e) {
    toast(String(e));
  }
}

async function cancelReminder(id) {
  try {
    applySavedStore(await invoke("reminder_cancel", { id }));
  } catch (e) {
    toast(String(e));
  }
}

async function checkReminders() {
  for (const reminder of dueReminders(savedState.reminders, Date.now())) {
    const notice = reminderNotice(reminder);
    invoke("notify", { ...notice, target: savedTarget(reminder.entry) }).catch(() => {});
    toast(notice.title);
    playSound("message");
    await cancelReminder(reminder.id);
  }
}

function syncSavedButton() {
  const now = Date.now();
  const upcoming = savedState.reminders.filter(r => r.due > now).length;
  const btn = $("saved-btn");
  btn.title = savedButtonTitle(savedState.reminders, now);
  const count = $("saved-count");
  count.textContent = pinBadgeText(upcoming);
  count.classList.toggle("hidden", !upcoming);
  const entry = document.querySelector(".saved-entry .dm-preview");
  if (entry) entry.textContent = savedSummary(savedState.saved, savedState.reminders, now);
}

function savedSidebarRow() {
  const row = el("div", "channel dm-thread saved-entry");
  if (savedState.open) row.classList.add("active");
  const mark = el("div", "avatar saved-entry-mark");
  mark.appendChild(svgIcon("bookmark", 18));
  row.appendChild(mark);
  const meta = el("div", "dm-meta");
  meta.appendChild(el("div", "dm-name", "Saved"));
  meta.appendChild(el("div", "dm-preview", savedSummary(savedState.saved, savedState.reminders, Date.now())));
  row.appendChild(meta);
  row.onclick = toggleSavedView;
  return row;
}

function toggleSavedView() {
  if (savedState.open) closeSavedView();
  else openSavedView();
}

function openSavedView() {
  closeFloating();
  savedState.open = true;
  $("saved-view").classList.remove("hidden");
  $("saved-btn").classList.add("active");
  for (const row of document.querySelectorAll(".saved-entry")) row.classList.add("active");
  renderSavedView();
}

function closeSavedView() {
  savedState.open = false;
  $("saved-view").classList.add("hidden");
  $("saved-btn").classList.remove("active");
  for (const row of document.querySelectorAll(".saved-entry")) row.classList.remove("active");
}

function savedSectionHead(label, count) {
  const head = el("div", "saved-section");
  head.appendChild(el("span", null, label));
  head.appendChild(el("span", null, String(count)));
  return head;
}

function renderSavedView() {
  const list = $("saved-list");
  list.replaceChildren();
  const now = Date.now();
  if (savedState.reminders.length) {
    list.appendChild(savedSectionHead("Reminders", savedState.reminders.length));
    for (const reminder of savedState.reminders) list.appendChild(savedRow(reminder.entry, { reminder, now }));
  }
  list.appendChild(savedSectionHead("Saved messages", savedState.saved.length));
  if (!savedState.saved.length) {
    list.appendChild(savedEmptyState());
    return;
  }
  for (const entry of savedState.saved) list.appendChild(savedRow(entry, { now }));
}

function savedEmptyState() {
  const empty = el("div", "saved-empty");
  const mark = el("div", "saved-empty-mark");
  mark.appendChild(svgIcon("bookmark", 26));
  empty.appendChild(mark);
  empty.appendChild(el("strong", null, "Nothing saved yet"));
  empty.appendChild(el("span", null, "Hover a message and choose Save to keep it here, or Remind Me to hear about it later. Saved messages stay on this device."));
  return empty;
}

function savedRow(entry, { reminder = null, now }) {
  const row = el("div", "saved-row");
  row.dataset.id = entry.message;
  row.setAttribute("role", "button");
  row.tabIndex = 0;
  row.appendChild(makeAvatar(entry.author_id, entry.author_name));
  const body = el("div", "saved-body");
  const line = el("div", "saved-line");
  line.appendChild(el("span", "saved-author", entry.author_name || "Someone"));
  line.appendChild(el("span", "saved-origin", savedOrigin(entry)));
  if (reminder) {
    const chip = el("span", "saved-chip" + (reminder.due <= now ? " due" : ""));
    chip.appendChild(svgIcon("bell", 14));
    chip.appendChild(el("span", null, reminderLabel(reminder.due, now)));
    line.appendChild(chip);
  } else {
    line.appendChild(el("span", "saved-time", formatTime(entry.created_at)));
  }
  body.appendChild(line);
  const text = savedExcerpt(entry, plainText);
  const excerpt = el("div", "saved-text", text);
  if (!entry.content) excerpt.classList.add("muted");
  body.appendChild(excerpt);
  row.appendChild(body);
  const remove = el("button", "saved-remove");
  remove.title = reminder ? "Cancel Reminder" : "Unsave";
  remove.setAttribute("aria-label", remove.title);
  remove.appendChild(svgIcon("close", 16));
  remove.onclick = async e => {
    e.stopPropagation();
    if (reminder) await cancelReminder(reminder.id);
    else await unsaveEntry(entry);
  };
  row.appendChild(remove);
  const jump = () => jumpToSaved(entry);
  row.onclick = jump;
  row.onkeydown = e => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      jump();
    }
  };
  return row;
}

async function unsaveEntry(entry) {
  try {
    applySavedStore(await invoke("saved_set", { entry, saved: false }));
    renderMessages(state.renderedMessages || []);
  } catch (e) {
    toast(String(e));
  }
}

async function jumpToSaved(entry) {
  closeSavedView();
  try {
    if (entry.dm) {
      if (state.view !== "dm") {
        setViewMode("dm");
        state.currentServer = null;
        state.currentChannel = null;
        renderServerRail();
      }
      if (!state.currentDm || state.currentDm.user !== entry.dm) await selectDm(entry.dm, entry.dm_name || "Direct Message");
      await jumpToMessage(entry.message);
    } else if (entry.server && entry.channel) {
      await openMessageLink({ server: entry.server, channel: entry.channel, message: entry.message });
    } else {
      toast("That message has no place to jump to");
    }
  } catch (e) {
    toast(String(e));
  }
}

function wireSaved() {
  $("saved-btn").onclick = toggleSavedView;
  $("saved-close").onclick = closeSavedView;
  document.addEventListener("click", e => {
    const pop = $("remind-pop");
    if (!pop.classList.contains("hidden") && !pop.contains(e.target) && !e.target.closest(".msg-actions, #msg-menu")) closeRemindPop();
    if (savedState.open && !$("saved-view").contains(e.target) && !e.target.closest("#saved-btn, .saved-entry, #modal-backdrop, .pop-menu")) closeSavedView();
  });
  document.addEventListener("keydown", e => {
    if (e.key !== "Escape") return;
    closeRemindPop();
    if (savedState.open) closeSavedView();
  });
}

let forwarding = null;

async function openForward(message) {
  forwarding = message;
  const preview = $("forward-preview");
  preview.replaceChildren();
  preview.appendChild(makeAvatar(message.author_id, message.author_name));
  const body = el("div", "msg-body");
  const head = el("div", "msg-head");
  head.appendChild(el("span", "msg-author", message.author_name));
  head.appendChild(el("span", "msg-time", formatTime(message.created_at)));
  body.appendChild(head);
  body.appendChild(el("div", "msg-content", plainText(message.content)));
  preview.appendChild(body);
  $("forward-files-note").classList.toggle("hidden", !(message.attachments || []).some(a => a.filename));
  $("forward-search").value = "";
  showModal("modal-forward");
  $("forward-search").focus();
  const views = [];
  for (const summary of state.servers) {
    try {
      views.push(await invoke("get_server", { serverId: summary.id }));
    } catch {
      continue;
    }
  }
  let dms = [];
  try {
    dms = await invoke("list_dm_threads");
  } catch {}
  forwarding.targets = { servers: views.map(s => ({ ...s, channels: s.channels.map(c => ({ ...c, name: channelPlainName(c) })) })), dms };
  renderForwardList();
}

function renderForwardList() {
  const list = $("forward-list");
  list.innerHTML = "";
  if (!forwarding || !forwarding.targets) {
    list.appendChild(el("div", "dm-list-empty", "Loading places to send to"));
    return;
  }
  const targets = forwardTargets({ ...forwarding.targets, query: $("forward-search").value });
  for (const target of targets) {
    const row = el("button", "person-row forward-row");
    if (target.kind === "dm") row.appendChild(avatarWithPresence(target.name, target.user));
    else if (target.kind === "group") row.appendChild(groupAvatar());
    else row.appendChild(el("span", "server-chip", initials(target.sub)));
    const meta = el("div", "dm-meta");
    meta.appendChild(el("div", "dm-name", target.name));
    meta.appendChild(el("div", "dm-preview", target.sub));
    row.appendChild(meta);
    row.onclick = () => sendForward(target);
    list.appendChild(row);
  }
  if (!targets.length) list.appendChild(el("div", "dm-list-empty", "Nowhere matches that name."));
}

function forwardOrigin() {
  if (inDmView()) return null;
  if (isGroupView()) return state.currentServer.name;
  return state.currentChannel ? `#${channelPlainName(state.currentChannel)}` : null;
}

async function sendForward(target) {
  const message = forwarding;
  if (!message) return;
  const content = forwardBody({ content: message.content, author: message.author_name, where: forwardOrigin() });
  try {
    if (target.kind === "dm") await invoke("send_dm", { userId: target.user, content, replyTo: null });
    else await invoke("send_message", { serverId: target.server, channelId: target.channel, content, replyTo: null, previewUrl: null });
    hideModals();
    forwarding = null;
    toast(`Forwarded to ${target.name}`);
  } catch (e) {
    toast(String(e));
  }
}

function imageSourceFromHtml(html) {
  if (!html) return null;
  const doc = new DOMParser().parseFromString(html, "text/html");
  const src = doc.querySelector("img")?.getAttribute("src") || "";
  return /^https?:\/\//i.test(src) ? src : null;
}

function keepScrolledOnLoad(img) {
  img.addEventListener("load", () => {
    const box = $("messages");
    const slack = img.offsetHeight + 120;
    if (box.scrollHeight - box.scrollTop - box.clientHeight <= slack) {
      box.scrollTop = box.scrollHeight;
    }
  });
}

const loadedImageLinks = new Set();

function wireExternalLink(a, url) {
  a.href = "#";
  a.onclick = e => {
    e.preventDefault();
    invoke("open_url", { url }).catch(err => toast(String(err)));
  };
  return a;
}

function wireOutsideLink(a, url, { labelled = false } = {}) {
  a.href = "#";
  a.onclick = e => {
    e.preventDefault();
    e.stopPropagation();
    openOutsideLink(url, labelled ? a.textContent : null);
  };
  return a;
}

function trustedLinkDomains() {
  try {
    return readTrustedDomains(localStorage.getItem(TRUSTED_LINKS_KEY));
  } catch {
    return [];
  }
}

function saveTrustedLinkDomains(domains) {
  try { localStorage.setItem(TRUSTED_LINKS_KEY, writeTrustedDomains(domains)); } catch {}
  renderTrustedLinkSettings();
}

function renderTrustedLinkSettings() {
  const domains = trustedLinkDomains();
  $("trusted-links-summary").textContent = trustedLinksSummary(domains);
  $("trusted-links-forget").disabled = domains.length === 0;
}

async function openOutsideLink(url, label = null) {
  let check;
  try {
    check = await invoke("check_link", { url, label, trusted: trustedLinkDomains() });
  } catch (err) {
    toast(String(err));
    return;
  }
  if (check.level === "open") openCheckedLink(check.url);
  else showLeavingPrompt(check);
}

function openCheckedLink(url) {
  invoke("open_url", { url }).catch(err => toast(String(err)));
}

const leaving = { check: null, returnTo: null };

function visibleModalId() {
  if ($("modal-backdrop").classList.contains("hidden")) return null;
  const open = [...document.querySelectorAll(".modal")].find(m => !m.classList.contains("hidden"));
  return open && open.id !== "modal-leaving" ? open.id : null;
}

function showLeavingPrompt(check) {
  const prompt = leavingPrompt(check);
  leaving.check = check;
  leaving.returnTo = visibleModalId();
  const modal = $("modal-leaving");
  modal.classList.toggle("warn", prompt.tone === "warn");
  modal.classList.toggle("block", prompt.tone === "block");
  $("leaving-title").textContent = prompt.title;
  $("leaving-intro").textContent = prompt.intro;
  $("leaving-host-prefix").textContent = prompt.host.prefix;
  $("leaving-host-site").textContent = prompt.host.site;
  $("leaving-spelled").textContent = prompt.spelled ? `Spelled out: ${prompt.spelled}` : "";
  $("leaving-spelled").classList.toggle("hidden", !prompt.spelled);
  $("leaving-notes").replaceChildren(...prompt.notes.map(note => el("li", null, note)));
  $("leaving-notes").classList.toggle("hidden", prompt.notes.length === 0);
  $("leaving-trust").checked = false;
  $("leaving-trust-row").classList.toggle("hidden", !prompt.canTrust);
  const go = $("leaving-go");
  go.textContent = prompt.openLabel;
  go.className = prompt.tone === "plain" ? "btn-primary" : "btn-danger";
  showModal("modal-leaving");
  $("leaving-cancel").focus();
}

function closeLeavingPrompt() {
  const back = leaving.returnTo;
  leaving.check = null;
  leaving.returnTo = null;
  if (back) showModal(back);
  else hideModals();
}

function confirmLeaving() {
  const check = leaving.check;
  if (!check) return;
  if (check.can_trust && $("leaving-trust").checked) saveTrustedLinkDomains(withTrustedDomain(trustedLinkDomains(), check.trust_key));
  closeLeavingPrompt();
  openCheckedLink(check.url);
}

function imageEmbeds(urls) {
  if (urls.length < 2) return urls.map(renderImageEmbed);
  const grid = el("div", imageGridClass(urls.length));
  for (const url of urls) grid.appendChild(renderImageEmbed(url));
  return [grid];
}

function renderLinkPreview(preview) {
  const layout = previewLayout(preview);
  const card = el("div", `link-card ${layout}`);
  const text = el("div", "link-card-text");
  const site = previewSite(preview);
  if (site) text.appendChild(el("div", "link-card-site", site));
  if (preview.title) {
    const title = wireOutsideLink(el("a", "link-card-title", preview.title), preview.url);
    title.title = `Open ${preview.url}`;
    text.appendChild(title);
  }
  if (preview.description) text.appendChild(el("div", "link-card-description", preview.description));
  card.appendChild(text);
  if (layout !== "text") card.appendChild(linkCardImage(preview, layout));
  return card;
}

function linkCardImage(preview, layout) {
  const image = preview.image;
  const box = previewImageBox(image, layout);
  const frame = el("div", "link-card-image");
  frame.style.width = `${box.width}px`;
  frame.style.aspectRatio = `${box.width} / ${box.height}`;
  const img = el("img");
  img.alt = "";
  img.onclick = () => openLightbox(img.src, preview.url, null);
  loadImagePreview({ blob: image.blob, provider: image.provider, mime: image.mime, size_bytes: image.size_bytes, shared: true }, false)
    .then(src => {
      img.src = src;
      frame.appendChild(img);
    })
    .catch(() => frame.classList.add("missing"));
  return frame;
}

function renderImageEmbed(url) {
  const host = imageHost(url) || url;
  const wrap = el("div", "image-embed");
  const source = wireOutsideLink(el("a", "embed-source", host), url);
  source.title = `Open ${url}`;
  wrap.appendChild(source);
  if (embedMode(url, mediaProxyOn()) === "auto" || loadedImageLinks.has(url)) {
    const img = embedImage(url);
    keepScrolledOnLoad(img);
    img.loading = "lazy";
    img.onerror = () => wrap.remove();
    img.src = viaAccountServer(url);
    wrap.appendChild(img);
  } else {
    wrap.appendChild(imageGate(url, host));
  }
  return wrap;
}

function embedImage(url) {
  const img = el("img", "chat-image");
  holdPlaceUntilLoaded(img);
  img.onclick = () => openLightbox(img.src, url, null);
  return img;
}

function holdPlaceUntilLoaded(img) {
  img.classList.add("waiting");
  img.addEventListener("load", () => img.classList.remove("waiting"), { once: true });
}

function imageGate(url, host) {
  const text = imageGateText(host);
  const gate = el("div", "image-gate");
  const lines = el("div", "image-gate-text");
  const title = el("div", "image-gate-title", text.title);
  lines.appendChild(title);
  lines.appendChild(el("div", "image-gate-hint", text.hint));
  const button = el("button", "btn-secondary btn-small image-gate-load", text.load);
  button.type = "button";
  button.onclick = () => {
    button.disabled = true;
    button.textContent = text.loading;
    const img = embedImage(url);
    img.onload = () => {
      loadedImageLinks.add(url);
      gate.replaceWith(img);
      img.scrollIntoView({ block: "nearest" });
    };
    img.onerror = () => {
      title.textContent = text.failed;
      button.textContent = text.retry;
      button.disabled = false;
    };
    img.src = viaAccountServer(url);
  };
  gate.appendChild(lines);
  gate.appendChild(button);
  return gate;
}

function openLightbox(src, sourceUrl, filename, att = null) {
  if (!src) return;
  $("lightbox-img").src = src;
  const open = $("lightbox-open");
  open.classList.toggle("hidden", !sourceUrl);
  open.onclick = e => {
    e.stopPropagation();
    closeLightbox();
    openOutsideLink(sourceUrl);
  };
  const save = $("lightbox-save");
  save.classList.toggle("hidden", !att);
  save.onclick = e => {
    e.stopPropagation();
    saveAttachment(att);
  };
  const source = $("lightbox-source");
  source.textContent = sourceUrl || filename || "";
  source.classList.toggle("hidden", !(sourceUrl || filename));
  source.classList.toggle("linked", Boolean(sourceUrl));
  source.onclick = e => {
    e.preventDefault();
    e.stopPropagation();
    if (!sourceUrl) return;
    closeLightbox();
    openOutsideLink(sourceUrl);
  };
  $("lightbox").classList.remove("hidden");
}

function closeLightbox() {
  $("lightbox").classList.add("hidden");
  $("lightbox-img").src = "";
}

const imagePreviewCache = new Map();
const imagePreviewLoads = new Map();
const imagePreviewFailures = new Map();

function loadImagePreview(att, clicked) {
  const cached = imagePreviewCache.get(att.blob);
  if (cached) return Promise.resolve(cached);
  let pending = imagePreviewLoads.get(att.blob);
  if (!pending) {
    pending = invoke("get_attachment_preview", {
      blob: att.blob,
      provider: att.provider ?? null,
      mime: att.mime,
      sizeBytes: att.size_bytes ?? null,
      clicked,
      shared: att.shared ?? false,
      serverId: att.server ?? null,
    })
      .then(dataUrl => {
        imagePreviewCache.set(att.blob, dataUrl);
        imagePreviewFailures.delete(att.blob);
        return dataUrl;
      })
      .catch(e => {
        imagePreviewFailures.set(att.blob, { error: String(e), at: Date.now() });
        throw e;
      })
      .finally(() => imagePreviewLoads.delete(att.blob));
    imagePreviewLoads.set(att.blob, pending);
  }
  return pending;
}

function chatImage(att, src = null) {
  const img = el("img", "chat-image");
  keepScrolledOnLoad(img);
  if (src) img.src = src;
  else holdPlaceUntilLoaded(img);
  img.alt = att.filename;
  img.onclick = () => openLightbox(img.src, null, att.filename, att);
  return img;
}

function renderInlineImage(att) {
  const cached = imagePreviewCache.get(att.blob);
  if (cached) return chatImage(att, cached);
  if (imagePreviewFailures.has(att.blob)) return missingImage(att);
  if (!loadsImageOnItsOwn(att)) return largeImageGate(att);
  const img = chatImage(att);
  loadImagePreview(att, false)
    .then(dataUrl => {
      img.src = dataUrl;
    })
    .catch(() => img.replaceWith(missingImage(att)));
  return img;
}

function missingImage(att) {
  const failure = imagePreviewFailures.get(att.blob);
  const text = missingImageText(failure && failure.error);
  const tile = el("div", "image-missing");
  tile.title = att.filename;
  const title = el("div", "image-missing-title", text.title);
  const hint = el("div", "image-missing-hint", text.hint);
  const button = el("button", "btn-secondary btn-small image-missing-retry", text.retry);
  button.type = "button";
  const retry = () => {
    button.disabled = true;
    button.textContent = text.loading;
    loadImagePreview(att, !loadsImageOnItsOwn(att))
      .then(dataUrl => tile.replaceWith(chatImage(att, dataUrl)))
      .catch(e => {
        const again = missingImageText(String(e));
        hint.textContent = again.hint;
        button.textContent = again.retry;
        button.disabled = false;
      });
  };
  button.onclick = e => {
    e.stopPropagation();
    retry();
  };
  tile.append(svgIcon("warning", 22), title, hint, button);
  if (imagePreviewLoads.has(att.blob) || retriesImageNow(failure, Date.now())) retry();
  return tile;
}

function largeImageGate(att) {
  const size = typeof att.size_bytes === "number" ? formatSize(att.size_bytes) : "size unknown";
  const text = largeImageGateText(size);
  const gate = el("div", "image-gate attachment-gate");
  const lines = el("div", "image-gate-text");
  const title = el("div", "image-gate-title", text.title);
  lines.append(title, el("div", "image-gate-hint", text.hint));
  const button = el("button", "btn-secondary btn-small image-gate-load", text.load);
  button.type = "button";
  const load = () => {
    button.disabled = true;
    button.textContent = text.loading;
    loadImagePreview(att, true)
      .then(dataUrl => {
        const img = chatImage(att, dataUrl);
        gate.replaceWith(img);
        img.scrollIntoView({ block: "nearest" });
      })
      .catch(() => {
        title.textContent = text.failed;
        button.textContent = text.retry;
        button.disabled = false;
      });
  };
  button.onclick = load;
  gate.append(lines, button);
  if (imagePreviewLoads.has(att.blob)) load();
  return gate;
}

const videoUrls = new Map();

function videoPlayer(url) {
  const video = el("video", "chat-video-player");
  video.src = url;
  video.controls = true;
  video.preload = "metadata";
  video.playsInline = true;
  return video;
}

function renderInlineVideo(att) {
  const box = el("div", "chat-video");
  const cached = videoUrls.get(att.blob);
  if (cached) {
    box.appendChild(videoPlayer(cached));
    return box;
  }
  const play = el("button", "chat-video-play");
  play.type = "button";
  play.title = "Play Video";
  play.appendChild(svgIcon("play", 30));
  const label = el("span", "chat-video-label", `Video, ${formatSize(att.size_bytes)}`);
  box.append(play, label);
  play.onclick = async () => {
    play.disabled = true;
    box.classList.add("loading");
    label.textContent = "Loading video";
    try {
      const bytes = await invoke("get_attachment_video", { blob: att.blob, provider: att.provider ?? null, sizeBytes: att.size_bytes ?? null, serverId: att.server ?? null });
      const url = URL.createObjectURL(new Blob([bytes], { type: att.mime }));
      videoUrls.set(att.blob, url);
      const player = videoPlayer(url);
      box.replaceChildren(player);
      player.play().catch(() => {});
    } catch (e) {
      play.disabled = false;
      box.classList.remove("loading");
      label.textContent = `Could not load the video: ${e}`;
    }
  };
  return box;
}

function renderShareButton() {
  const btn = $("share-btn");
  btn.classList.toggle("active", state.sharing);
  btn.title = state.sharing ? "Stop sharing your screen" : "Share your screen";
  btn.replaceChildren(svgIcon("screen", 16), el("span", null, state.sharing ? "Stop Sharing" : "Share Screen"));
  renderVoicePanel();
}

function renderVoicePanel() {
  const panel = $("voice-panel");
  const vs = state.voiceServer;
  panel.classList.toggle("hidden", !(state.voiceChannelId && vs));
  syncSignalPolling(Boolean(state.voiceChannelId && vs));
  if (!state.voiceChannelId || !vs) return;

  const call = vs.call ? state.call : null;
  $("vp-status-text").textContent = call && call.state !== "active" ? "Calling" : "Voice Connected";
  $("vp-where").textContent = call ? `Call with ${vs.name}` : `${vs.channelName} / ${vs.name}`;
  renderVoiceTopic(call ? null : vs);
  $("vp-camera").classList.toggle("hidden", Boolean(call));
  renderVoiceInviteButton(Boolean(call));
  const playing = !state.sharing && state.user && state.user.activity && state.user.activity.startsWith("Playing ")
    ? state.user.activity.slice(8)
    : null;
  const gameBtn = $("vp-game-share");
  gameBtn.classList.toggle("hidden", !playing || Boolean(call));
  if (playing) {
    gameBtn.replaceChildren(svgIcon("screen", 16), el("span", null, `Share ${playing}`));
    gameBtn.title = `Stream ${playing} to your voice channel at 60 fps`;
  }
  $("vp-signal").replaceChildren(svgIcon("signal", 14));
  setIcon($("vp-disconnect"), "hangup", 20);

  $("vp-camera-icon").replaceChildren(svgIcon(state.cameraOn ? "camera" : "cameraOff", 18));
  $("vp-camera").classList.toggle("active", Boolean(state.cameraOn));
  $("vp-camera").title = state.cameraOn ? "Turn Off Camera" : "Turn On Camera";
  renderCamGrid();

  renderVoicePanelActivity();
  $("vp-live").classList.toggle("hidden", !state.sharing);
  $("vp-live-source").textContent = state.shareSource ? `Streaming ${state.shareSource}` : "";
  $("vp-live-title").textContent = `Live in ${vs.channelName}`;

  $("vp-watch").classList.toggle("hidden", !state.watchingUser);
  $("vp-watch-label").textContent = state.watchingUser ? `Watching ${state.watchingName || "a stream"}` : "";

  const streams = $("vp-streams");
  streams.innerHTML = "";
  const server = state.currentServer && state.currentServer.id === vs.id ? state.currentServer : null;
  renderStageStrip(server);
  const others = server
    ? (server.screen_shares || []).filter(s => (!state.user || s.user !== state.user.id) && s.user !== state.watchingUser)
    : [];
  for (const share of others) {
    const row = el("div", "vp-stream");
    const head = el("div", "vp-stream-head");
    head.appendChild(el("span", "live-badge", "LIVE"));
    head.appendChild(el("span", "vp-stream-name", `${share.name} is sharing their screen`));
    row.appendChild(head);
    const watch = el("button", "vp-pill vp-stream-watch", "Watch Stream");
    watch.onclick = () => watchScreen(share.user, share.name);
    row.appendChild(watch);
    streams.appendChild(row);
  }
}

function renderStageStrip(server) {
  const strip = $("vp-stage");
  const channel = server && !server.group ? server.channels.find(c => c.id === state.voiceChannelId) : null;
  const hand = channel && state.user ? handState(channel, state.user.id, Boolean(server.can_manage_stage)) : null;
  strip.classList.toggle("hidden", !hand);
  if (!hand) return;
  $("vp-stage-icon").replaceChildren(svgIcon(hand.role === "speaker" ? "mic" : "hand", 14));
  $("vp-stage-text").textContent = hand.text;
  $("vp-stage-text").title = hand.detail;
  const button = $("vp-hand");
  button.textContent = hand.label;
  button.title = hand.title;
  button.classList.toggle("on", hand.raised);
  button.classList.toggle("spent", hand.role === "speaker");
  button.disabled = hand.role === "speaker";
  button.onclick = () => toggleHand(!hand.raised);
}
const ACTIVITY_REMOTE_SETTLE_MS = 400;
const ACTIVITY_DRIFT_CHECK_MS = 2000;

const activityUi = { run: null, open: false, seq: 0, pick: "tictactoe", chip: null, videoBlob: null, settling: 0 };

function activityMe() {
  return state.user ? state.user.id : null;
}

function activityMemberName(id) {
  if (id === activityMe()) return state.user.display_name || state.user.username || "You";
  const members = (state.currentServer && state.currentServer.members) || [];
  const member = members.find(m => m.id === id);
  return member ? member.nickname || member.name : "Someone";
}

function activityChannelView() {
  const server = state.currentServer;
  if (!server || !state.voiceServer || server.id !== state.voiceServer.id || !state.voiceChannelId) return null;
  return server.channels.find(c => c.id === state.voiceChannelId) || null;
}

function reconcileActivity() {
  if (!state.voiceChannelId || !state.voiceServer) {
    if (activityUi.run) endActivityLocally(null);
    renderActivityUi();
    return;
  }
  const channel = activityChannelView();
  if (!channel) {
    renderActivityUi();
    return;
  }
  const current = channel.activity || null;
  if (current && (!activityUi.run || activityUi.run.id !== current.id)) adoptActivity(current, null);
  else if (!current && activityUi.run) endActivityLocally(null);
  else renderActivityUi();
}

function adoptActivity(activity, by) {
  releaseActivityVideo();
  activityUi.run = newActivityRun(activity);
  const me = activityMe();
  activityUi.open = activity.starter === me;
  if (by && by !== me) toast(`${activityMemberName(by)} started ${activityName(activity)}`);
  renderActivityUi();
}

function endActivityLocally(by) {
  const was = activityUi.run;
  activityUi.run = null;
  activityUi.open = false;
  releaseActivityVideo();
  if (was && by && by !== activityMe()) toast(`${activityMemberName(by)} ended ${activityName(was.activity)}`);
  renderActivityUi();
}

function onActivitySet(payload) {
  if (!state.voiceServer || payload.server !== state.voiceServer.id || payload.channel !== state.voiceChannelId) return;
  const run = activityUi.run;
  if (payload.activity) {
    if (run && run.id === payload.activity.id) return;
    adoptActivity(payload.activity, payload.by);
  } else if (run) {
    endActivityLocally(payload.by);
  }
}

function onActivityInput(payload) {
  const run = activityUi.run;
  if (!run || !state.voiceServer || payload.server !== state.voiceServer.id || payload.channel !== state.voiceChannelId) return;
  const result = applyActivityInput(run, payload.input, payload.by, payload.seq);
  if (!result.ok) return;
  if (payload.input === "join" && run.starter === activityMe()) sendActivityInput(syncInput(run, Date.now()), false);
  if (payload.input && payload.input.playback) syncActivityVideo();
  if (payload.input && payload.input.sync && run.kind === "watch") syncActivityVideo();
  renderActivityUi();
}

async function sendActivityInput(input, applyLocally = true) {
  const run = activityUi.run;
  if (!run || !state.voiceServer) return;
  if (applyLocally) {
    const result = applyActivityInput(run, input, activityMe(), null);
    if (!result.ok) {
      if (result.reason) toast(result.reason);
      return;
    }
    renderActivityUi();
  }
  const seq = ++activityUi.seq;
  try {
    await invoke("send_activity_input", { serverId: state.voiceServer.id, seq, input });
  } catch (e) {
    toast(String(e));
  }
}

async function startActivity(activity) {
  if (!state.voiceServer) return;
  try {
    await invoke("set_activity", { serverId: state.voiceServer.id, activity });
    hideModals();
    adoptActivity(activity, activityMe());
    activityUi.open = true;
    renderActivityUi();
    if (state.currentServer && state.currentServer.id === state.voiceServer.id) await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

async function endActivity() {
  if (!activityUi.run || !state.voiceServer) return;
  try {
    await invoke("set_activity", { serverId: state.voiceServer.id, activity: null });
    endActivityLocally(activityMe());
    if (state.currentServer && state.currentServer.id === state.voiceServer.id) await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

function toggleActivityPanel(open) {
  activityUi.open = open === undefined ? !activityUi.open : open;
  renderActivityUi();
}

function renderActivityUi() {
  renderActivityPanel();
  renderVoicePanelActivity();
}

function voiceServerAllowsActivities() {
  const vs = state.voiceServer;
  if (!vs || !vs.id) return false;
  const current = state.currentServer && state.currentServer.id === vs.id ? state.currentServer : null;
  return Boolean(current ? current.activities_enabled : vs.activities_enabled);
}

function renderVoicePanelActivity() {
  const btn = $("vp-activities");
  if (!btn) return;
  const run = activityUi.run;
  const vs = state.voiceServer;
  btn.classList.toggle("hidden", !vs || Boolean(vs.call) || (!run && !voiceServerAllowsActivities()));
  btn.classList.toggle("running", Boolean(run));
  const info = run ? activityKindInfo(run.activity) : null;
  btn.replaceChildren(svgIcon(info ? info.icon : "gamepad", 16), el("span", null, run ? `Open ${activityName(run.activity)}` : "Activities"));
  btn.title = run ? `${activityName(run.activity)} is running in your voice channel` : "Start something together with everyone in the channel";
  btn.onclick = () => (run ? toggleActivityPanel() : openActivityPicker());
}

function renderActivityPanel() {
  const panel = $("activity-panel");
  const run = activityUi.run;
  const shown = Boolean(run && activityUi.open);
  panel.classList.toggle("hidden", !shown);
  if (!shown) return;
  const me = activityMe();
  const info = activityKindInfo(run.activity);
  $("activity-icon").replaceChildren(svgIcon(info ? info.icon : "gamepad", 20));
  $("activity-title").textContent = activityName(run.activity);
  $("activity-sub").textContent = `Started by ${activityMemberName(run.starter)}`;
  renderActivityPeople(run);
  const joined = isParticipant(run, me);
  const starter = run.starter === me;
  $("activity-join").classList.toggle("hidden", joined);
  $("activity-leave").classList.toggle("hidden", !joined);
  $("activity-leave").classList.toggle("ghost", starter);
  $("activity-leave").disabled = starter;
  const mayEnd = canEndActivity(run, me, state.currentServer);
  $("activity-end").classList.toggle("ghost", !mayEnd);
  $("activity-end").disabled = !mayEnd;
  setIcon($("activity-close"), "chevronDown", 18);
  $("activity-game").classList.toggle("hidden", run.kind !== "tictactoe");
  $("activity-watch").classList.toggle("hidden", run.kind !== "watch");
  if (run.kind === "tictactoe") renderTicTacToe(run, me);
  else renderActivityWatch(run, me);
}

function renderActivityPeople(run) {
  const box = $("activity-people");
  box.innerHTML = "";
  const { shown, more } = avatarRow(activityParticipants(run));
  for (const id of shown) {
    const avatar = makeAvatar(id, activityMemberName(id), "avatar avatar-sm");
    const mark = markOf(run, id);
    if (mark) avatar.classList.add("player", mark === MARK_X ? "mark-x" : "mark-o");
    avatar.title = mark ? `${activityMemberName(id)} (${MARK_GLYPHS[mark]})` : activityMemberName(id);
    box.appendChild(avatar);
  }
  if (more) box.appendChild(el("span", "activity-people-more", `+${more}`));
}

function renderTicTacToe(run, me) {
  const status = gameStatus(run, me, activityMemberName);
  $("activity-turn").textContent = status.text;
  const board = $("ttt-board");
  if (board.children.length !== BOARD_CELLS) {
    board.innerHTML = "";
    for (let i = 0; i < BOARD_CELLS; i++) {
      const cell = el("button", "ttt-cell");
      cell.type = "button";
      cell.dataset.cell = String(i);
      cell.setAttribute("role", "gridcell");
      cell.onclick = () => {
        if (canMoveAt(activityUi.run, activityMe(), i)) sendActivityInput({ move: { cell: i } });
      };
      board.appendChild(cell);
    }
  }
  const line = status.line || [];
  for (const cell of board.children) {
    const i = Number(cell.dataset.cell);
    const mark = run.board[i];
    cell.textContent = MARK_GLYPHS[mark] || "";
    cell.classList.toggle("o", mark === MARK_O);
    cell.classList.toggle("x", mark === MARK_X);
    cell.classList.toggle("win", line.includes(i));
    cell.classList.toggle("open", canMoveAt(run, me, i));
    cell.setAttribute("aria-label", mark ? `Cell ${i + 1}, ${MARK_GLYPHS[mark]}` : `Cell ${i + 1}, empty`);
  }
  $("ttt-rematch").classList.toggle("hidden", !status.canRematch);
  const hint = $("ttt-hint");
  if (status.canRematch) hint.textContent = "";
  else if (status.over) hint.textContent = "Waiting for a rematch";
  else if (run.players.length < 2) hint.textContent = isPlayer(run, me) ? "Share the voice channel to find an opponent" : "The first to join plays O";
  else hint.textContent = isPlayer(run, me) ? `You are ${MARK_GLYPHS[markOf(run, me)]}` : "You are watching";
}

function renderActivityWatch(run, me) {
  $("activity-watch-title").textContent = watchTitle(run.activity);
  const video = $("activity-video");
  video.controls = isParticipant(run, me);
  loadActivityVideo(run);
  renderActivitySyncLabel(run);
}

function renderActivitySyncLabel(run) {
  const label = $("activity-sync");
  const video = $("activity-video");
  if (!video.src || !isParticipant(run, activityMe())) {
    label.textContent = isParticipant(run, activityMe()) ? "" : "Join to control playback";
    label.className = "activity-sync";
    return;
  }
  const off = watchNeedsSeek(video.currentTime, watchTargetPosition(run.playback, Date.now()));
  label.textContent = off ? "Catching up" : "In sync";
  label.className = `activity-sync ${off ? "catching" : "good"}`;
}

async function loadActivityVideo(run) {
  const video = $("activity-video");
  const url = watchLinkUrl(run.activity);
  if (url) {
    if (video.src !== url) {
      video.src = url;
      activityUi.videoBlob = null;
    }
    return;
  }
  const att = watchAttachment(run.activity);
  if (!att || activityUi.videoBlob === att.blob) return;
  activityUi.videoBlob = att.blob;
  const cached = videoUrls.get(att.blob);
  if (cached) {
    video.src = cached;
    return;
  }
  $("activity-video-wait").classList.remove("hidden");
  try {
    const bytes = await invoke("get_attachment_video", { blob: att.blob, provider: att.provider ?? null, sizeBytes: att.size_bytes ?? null });
    const objectUrl = URL.createObjectURL(new Blob([bytes], { type: att.mime }));
    videoUrls.set(att.blob, objectUrl);
    if (activityUi.run && activityUi.run.id === run.id) video.src = objectUrl;
  } catch (e) {
    toast(`Could not load the video: ${e}`);
  } finally {
    $("activity-video-wait").classList.add("hidden");
  }
}

function releaseActivityVideo() {
  const video = $("activity-video");
  if (!video) return;
  activityUi.settling = performance.now() + ACTIVITY_REMOTE_SETTLE_MS;
  video.pause();
  video.removeAttribute("src");
  video.load();
  activityUi.videoBlob = null;
}

function syncActivityVideo() {
  const run = activityUi.run;
  const video = $("activity-video");
  if (!run || run.kind !== "watch" || !video.src) return;
  const target = watchTargetPosition(run.playback, Date.now());
  activityUi.settling = performance.now() + ACTIVITY_REMOTE_SETTLE_MS;
  if (Number.isFinite(video.duration) && watchNeedsSeek(video.currentTime, target)) video.currentTime = target;
  if (run.playback.playing && video.paused) video.play().catch(() => {});
  else if (!run.playback.playing && !video.paused) video.pause();
  renderActivitySyncLabel(run);
}

function onActivityVideoChange() {
  const run = activityUi.run;
  const video = $("activity-video");
  if (!run || run.kind !== "watch" || performance.now() < activityUi.settling) return;
  if (!isParticipant(run, activityMe()) || !video.src) return;
  sendActivityInput({ playback: { playing: !video.paused, position: video.currentTime, at: Date.now() } });
}

function openActivityPicker() {
  if (!state.voiceServer || !state.voiceChannelId) {
    toast("Join a voice channel first");
    return;
  }
  activityUi.pick = "tictactoe";
  activityUi.chip = null;
  $("activity-link-input").value = "";
  renderActivityPicker();
  showModal("modal-activities");
}

function renderActivityPicker() {
  const cards = $("activity-cards");
  cards.innerHTML = "";
  for (const kind of ACTIVITY_KINDS) {
    const card = el("button", "activity-card");
    card.type = "button";
    card.setAttribute("role", "radio");
    card.setAttribute("aria-checked", String(kind.key === activityUi.pick));
    card.appendChild(el("span", "activity-card-icon")).appendChild(svgIcon(kind.icon, 22));
    card.appendChild(el("span", "activity-card-name", kind.name));
    card.appendChild(el("span", "activity-card-blurb", kind.blurb));
    card.onclick = () => {
      activityUi.pick = kind.key;
      renderActivityPicker();
    };
    cards.appendChild(card);
  }
  const watch = activityUi.pick === "watch";
  $("activity-setup-game").classList.toggle("hidden", watch);
  $("activity-setup-watch").classList.toggle("hidden", !watch);
  const chips = $("activity-chips");
  chips.innerHTML = "";
  for (const att of channelVideoAttachments(state.renderedMessages)) {
    const chip = el("button", "activity-chip");
    chip.type = "button";
    chip.setAttribute("aria-pressed", String(activityUi.chip && activityUi.chip.blob === att.blob));
    chip.appendChild(svgIcon("movie", 14));
    chip.appendChild(el("span", null, att.filename));
    chip.title = `${att.filename}, ${formatSize(att.size_bytes)}`;
    chip.onclick = () => {
      activityUi.chip = activityUi.chip && activityUi.chip.blob === att.blob ? null : att;
      if (activityUi.chip) $("activity-link-input").value = "";
      renderActivityPicker();
    };
    chips.appendChild(chip);
  }
  $("activity-start").textContent = watch ? "Start watching" : "Start game";
}

function pickedActivity() {
  const now = Date.now();
  const id = crypto.randomUUID();
  if (activityUi.pick !== "watch") return makeActivity("tictactoe", activityMe(), now, id);
  const url = $("activity-link-input").value.trim();
  if (url) {
    if (!isVideoLink(url)) throw new Error("Paste a direct https link that ends in .mp4, .webm or .ogg");
    return makeActivity("watch", activityMe(), now, id, watchLinkSource(url));
  }
  if (activityUi.chip) return makeActivity("watch", activityMe(), now, id, watchAttachmentSource(activityUi.chip));
  throw new Error("Paste a video link or pick an attached video");
}

function initActivityUi() {
  $("activity-cancel").onclick = hideModals;
  $("activity-start").onclick = async () => {
    try {
      await startActivity(pickedActivity());
    } catch (e) {
      toast(e.message || String(e));
    }
  };
  $("activity-link-input").addEventListener("input", () => {
    if ($("activity-link-input").value.trim()) {
      activityUi.chip = null;
      renderActivityPicker();
    }
  });
  $("activity-join").onclick = () => sendActivityInput("join");
  $("activity-leave").onclick = () => {
    sendActivityInput("leave");
    toggleActivityPanel(false);
  };
  $("activity-end").onclick = endActivity;
  $("activity-close").onclick = () => toggleActivityPanel(false);
  $("ttt-rematch").onclick = () => sendActivityInput("rematch");
  const video = $("activity-video");
  for (const name of ["play", "pause", "seeked"]) video.addEventListener(name, onActivityVideoChange);
  video.addEventListener("loadedmetadata", syncActivityVideo);
  setInterval(() => {
    const run = activityUi.run;
    if (run && run.kind === "watch" && run.playback.playing) syncActivityVideo();
  }, ACTIVITY_DRIFT_CHECK_MS);
}

async function disconnectVoice() {
  if (!state.voiceServer) return;
  if (state.voiceServer.call) return endDmCall();
  try {
    await invoke("set_voice_channel", { serverId: state.voiceServer.id, channelId: null });
    state.voiceChannelId = null;
    clearSpeaking();
    stopCameraTracks();
    state.cameraOn = false;
    cam.watching.clear();
    state.voiceServer = null;
    state.sharing = false;
    state.shareSource = null;
    if (state.watchingUser) await stopWatching();
    if (state.stageMode === "self") closeStage();
    toast("Disconnected from voice");
    playSound("leave");
    renderShareButton();
    if (state.currentServer) await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

function renderUserPanel() {
  if (!state.user) return;
  $("me-name").textContent = state.user.display_name || state.user.username;
  const presence = state.user.presence || "online";
  $("me-avatar").replaceChildren();
  paintAvatar($("me-avatar"), state.user.id, state.user.display_name || state.user.username);
  $("me-avatar").appendChild(el("span", "presence-dot " + presence));
  const back = backLine(state.user);
  const label = presence === "invisible" ? "Appearing offline" : (back || PRESENCE_LABELS[presence] || "Online");
  $("me-status").textContent = label;
  $("me-status").classList.toggle("back", Boolean(back));
  $("mute-btn").classList.toggle("danger", state.voice.muted);
  $("deafen-btn").classList.toggle("danger", state.voice.deafened);
  setIcon($("mute-btn"), state.voice.muted ? "micOff" : "mic");
  setIcon($("deafen-btn"), state.voice.deafened ? "headsetOff" : "headset");
  setIcon($("settings-btn"), "gear");
  const v = voiceInputState(state.voice);
  const talkTitle = talkKeyTitle(v.mode, v.talkKey, Boolean(state.talking));
  $("mute-btn").title = state.voice.muted ? "Unmute" : talkTitle || "Mute";
  $("mute-btn").classList.toggle("talking", Boolean(state.talking) && !state.voice.muted);
  $("deafen-btn").title = state.voice.deafened ? "Undeafen" : "Deafen";
  if (state.currentServer && state.voiceChannelId && state.view === "server") renderChannels();
}

async function refreshServers() {
  state.servers = await invoke("list_servers");
  for (const id of [...friendPresenceViews.keys()]) {
    if (!state.servers.some(s => s.id === id)) friendPresenceViews.delete(id);
  }
  loadShareableEmojis();
  renderServerRail();
}

async function refreshServersIfChanged() {
  const servers = await invoke("list_servers");
  if (JSON.stringify(servers) !== JSON.stringify(state.servers)) await refreshServers();
}

function readMembersPref() {
  try {
    return localStorage.getItem("commy.members.collapsed");
  } catch {
    return null;
  }
}

function readSidebarWidth() {
  try {
    const raw = Number(localStorage.getItem("commy.sidebar.width"));
    return Number.isFinite(raw) && raw > 0 ? raw : null;
  } catch {
    return null;
  }
}

function paintSidebarWidth(width) {
  const app = $("app");
  if (width == null) app.style.removeProperty("--sidebar-w");
  else app.style.setProperty("--sidebar-w", `${clampSidebarWidth(width, window.innerWidth)}px`);
}

function setupSidebarResize() {
  const handle = $("sidebar-resize");
  paintSidebarWidth(readSidebarWidth());
  window.addEventListener("resize", () => paintSidebarWidth(readSidebarWidth()));
  handle.addEventListener("pointerdown", e => {
    e.preventDefault();
    handle.setPointerCapture(e.pointerId);
    handle.classList.add("dragging");
    document.body.classList.add("sidebar-resizing");
    const left = $("app").querySelector(".sidebar").getBoundingClientRect().left;
    let width = null;
    const move = ev => {
      width = clampSidebarWidth(ev.clientX - left, window.innerWidth);
      paintSidebarWidth(width);
    };
    const done = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", done);
      handle.removeEventListener("pointercancel", done);
      handle.classList.remove("dragging");
      document.body.classList.remove("sidebar-resizing");
      if (width != null) {
        try {
          localStorage.setItem("commy.sidebar.width", String(width));
        } catch {}
      }
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", done);
    handle.addEventListener("pointercancel", done);
  });
  handle.addEventListener("dblclick", () => {
    try {
      localStorage.removeItem("commy.sidebar.width");
    } catch {}
    paintSidebarWidth(null);
  });
}

function applyMembersLayout() {
  const collapsed = membersCollapsed(readMembersPref(), window.innerWidth);
  $("app").classList.toggle("members-collapsed", collapsed);
  const btn = $("members-btn");
  btn.classList.toggle("active", !collapsed);
  btn.setAttribute("aria-pressed", String(!collapsed));
  btn.title = collapsed ? "Show Members" : "Hide Members";
}

function narrowSplit() {
  return isPhoneWidth(window.innerWidth) && !document.body.classList.contains("mobile-app") && window.matchMedia("(pointer: fine)").matches;
}

function narrowViewServer() {
  return state.view === "server" && state.currentServer && !state.currentServer.group ? state.currentServer : null;
}

function paintNarrowServerIcon(icon, server) {
  icon.replaceChildren();
  icon.style.background = "";
  if (!server) {
    icon.appendChild(el("img")).src = "images/commy_v3_noback.png";
    return;
  }
  paintServerIcon(icon, server);
  icon.style.background = serverTint(server.id);
}

function paintNarrowRail() {
  if (!$("narrow-active")) return;
  const server = narrowViewServer();
  paintNarrowServers(server);
  $("narrow-column").replaceChildren(narrowChannelTile(server), ...(server ? narrowVoiceTiles(server) : []));
}

function paintNarrowServers(server) {
  const active = $("narrow-active");
  paintNarrowServerIcon(active.querySelector(".narrow-active-icon"), server);
  active.title = server ? `${server.name}: server controls` : "Direct Messages";
  const others = railServers().filter(s => !server || s.id !== server.id);
  const shown = others.length > 4 ? others.slice(0, 3) : others;
  const cells = shown.map(other => {
    const cell = el("span", "narrow-grid-cell");
    paintNarrowServerIcon(cell, other);
    return cell;
  });
  if (others.length > shown.length) cells.push(el("span", "narrow-grid-cell narrow-grid-more", `+${others.length - shown.length}`));
  const grid = $("narrow-grid");
  grid.replaceChildren(...cells);
  grid.title = others.length ? `Switch server: ${others.map(s => s.name).join(", ")}` : "Switch server";
  grid.classList.toggle("unread", others.some(s => serverUnreadCount(s.id) > 0));
}

function narrowChannelTile(server) {
  const channel = state.currentChannel;
  const tile = el("button", "narrow-channel");
  tile.id = "narrow-channel";
  tile.title = "Switch channel";
  const mark = el("span", "narrow-channel-mark");
  mark.appendChild(server && channel ? channelMark(channel) : svgIcon("chat", 18));
  const name = el("span", "narrow-channel-name");
  if (server && channel) name.appendChild(channelNameNode(channel));
  else name.textContent = state.currentDm ? state.currentDm.name : "Messages";
  tile.append(mark, name, svgIcon("chevronDown", 12));
  tile.classList.toggle("unread", Boolean(server) && server.channels.some(c => (!channel || c.id !== channel.id) && shownUnread(server.id, c.id)));
  tile.onclick = () => toggleNarrowChannels(tile);
  return tile;
}

function toggleNarrowChannels(tile) {
  const app = $("app");
  app.classList.remove("flyout-open");
  const open = app.classList.toggle("channels-open");
  if (open) $("channel-list").style.top = `${Math.round(tile.getBoundingClientRect().top)}px`;
}

function narrowVoiceTiles(server) {
  return server.channels
    .filter(c => isVoiceKind(c.kind) && (c.id === state.voiceChannelId || voiceRingEntries(c).length))
    .map(narrowVoiceTile);
}

function narrowVoiceTile(channel) {
  const connected = channel.id === state.voiceChannelId;
  const tile = el("button", "narrow-voice-tile" + (connected ? " connected" : ""));
  tile.title = connected ? `You are in ${channel.name}` : `Join ${channel.name}`;
  const grid = el("span", "narrow-voice-grid");
  grid.append(...voiceRingCells(channel));
  const head = el("span", "narrow-voice-head");
  head.append(voiceKindIcon(channel.kind, 12), el("span", "narrow-voice-name", channel.name));
  tile.append(grid, head);
  tile.onclick = () => {
    if (connected) return;
    if (voiceFull(channel)) toast("That voice channel is full");
    else toggleVoice(channel);
  };
  return tile;
}

function narrowMenuRow(label, paintIcon, onPick, active = false) {
  const row = el("button", "narrow-menu-row" + (active ? " active" : ""));
  const icon = el("span", "narrow-menu-icon");
  paintIcon(icon);
  row.append(icon, el("span", "narrow-menu-name", label));
  row.onclick = () => {
    $("app").classList.remove("flyout-open");
    onPick();
  };
  return row;
}

function narrowGlyph(name) {
  return icon => icon.appendChild(svgIcon(name, 18));
}

function serverSwitchRows() {
  const current = narrowViewServer();
  return [
    narrowMenuRow("Direct Messages", icon => paintNarrowServerIcon(icon, null), () => $("home-btn").click(), !current),
    ...railServers().map(server => narrowMenuRow(server.name, icon => paintNarrowServerIcon(icon, server), () => selectServer(server.id), Boolean(current) && server.id === current.id)),
    narrowMenuRow("Add a Server", narrowGlyph("plus"), () => $("add-server-btn").click()),
    narrowMenuRow("Discover Servers", narrowGlyph("globe"), () => $("discover-btn").click()),
  ];
}

function serverControlRows(server) {
  const events = upcomingEventCount(server);
  const rows = [
    el("div", "narrow-menu-title", server.name),
    narrowMenuRow(events ? `${events} Event${events === 1 ? "" : "s"}` : "Events", narrowGlyph("calendar"), () => openEvents()),
  ];
  if (server.rules) rows.push(narrowMenuRow("Server Rules", narrowGlyph("book"), () => openRules()));
  if (selfAssignableRoles(server).length) rows.push(narrowMenuRow("Roles", narrowGlyph("roles"), () => openChooseRoles()));
  rows.push(el("div", "narrow-menu-sep"));
  if (server.can_invite) rows.push(narrowMenuRow("Invite People", narrowGlyph("friendAdd"), () => $("invite-btn").click()));
  rows.push(narrowMenuRow("Server Settings", narrowGlyph("gear"), () => $("server-settings-btn").click()));
  return rows;
}

function toggleNarrowMenu(kind, anchor) {
  const app = $("app");
  const menu = $("narrow-menu");
  app.classList.remove("channels-open");
  if (app.classList.contains("flyout-open") && menu.dataset.kind === kind) {
    app.classList.remove("flyout-open");
    return;
  }
  const server = narrowViewServer();
  menu.dataset.kind = kind;
  menu.replaceChildren(...(kind === "controls" && server ? serverControlRows(server) : serverSwitchRows()));
  menu.style.top = `${Math.round(anchor.getBoundingClientRect().top)}px`;
  app.classList.add("flyout-open");
}

function applyNarrowLayout() {
  const app = $("app");
  const split = narrowSplit();
  if (app.classList.contains("narrow-split") === split) return;
  app.classList.toggle("narrow-split", split);
  app.classList.remove("flyout-open", "channels-open");
  const panel = document.querySelector(".user-panel");
  if (split) {
    $("narrow-dock").appendChild(panel);
    app.dataset.pane = "split";
  } else {
    $("rail").appendChild(panel);
    app.dataset.pane = "list";
  }
  if (state.currentServer) renderChannels();
}

function wireNarrowLayout() {
  const sidebar = document.querySelector(".sidebar");
  const active = el("button", "narrow-active");
  active.id = "narrow-active";
  active.appendChild(el("span", "narrow-active-icon"));
  active.onclick = () => toggleNarrowMenu(narrowViewServer() ? "controls" : "servers", active);
  const grid = el("button", "narrow-grid");
  grid.id = "narrow-grid";
  grid.onclick = () => toggleNarrowMenu("servers", grid);
  const servers = el("div", "narrow-servers");
  servers.append(active, grid);
  sidebar.prepend(servers);
  const column = el("div", "narrow-column");
  column.id = "narrow-column";
  $("channel-list").after(column);
  const menu = el("div", "narrow-menu");
  menu.id = "narrow-menu";
  const dock = el("div", "narrow-dock");
  dock.id = "narrow-dock";
  sidebar.append(menu, dock);
  document.addEventListener("pointerdown", e => {
    if (!e.target.closest(".narrow-menu, .narrow-active, .narrow-grid")) $("app").classList.remove("flyout-open");
    if (!e.target.closest(".channel-scroll, .narrow-channel")) $("app").classList.remove("channels-open");
  });
  $("channel-list").addEventListener("click", e => {
    if (e.target.closest(".channel:not(.voice-channel)") && !e.target.closest("button")) $("app").classList.remove("channels-open");
  });
  window.addEventListener("resize", applyNarrowLayout);
  applyNarrowLayout();
}

function phonePane() {
  return $("app").dataset.pane || "list";
}

function setPhonePane(pane, { fromHistory = false } = {}) {
  if (narrowSplit()) return;
  const was = phonePane();
  if (was === pane) return;
  $("app").dataset.pane = pane;
  if (!fromHistory && phonePaneGoesDeeper(was, pane)) history.pushState({ pane }, "");
}

function phoneStep(action) {
  if (!isPhoneWidth(window.innerWidth) || narrowSplit()) return false;
  const from = phonePane();
  const to = phonePaneAfter(from, action);
  if (!phonePaneGoesDeeper(from, to) && history.state && history.state.pane) history.back();
  else setPhonePane(to);
  return true;
}

function phoneShowChat() {
  if (!isPhoneWidth(window.innerWidth) || narrowSplit()) return;
  const pane = phonePane();
  if (pane === "people") phoneStep("back");
  else if (pane === "list") phoneStep("open");
}

function settingsPhoneView(layer, view) {
  layer.dataset.phoneView = view;
  const title = layer.querySelector(".settings-phone-title");
  if (!title) return;
  const active = layer.querySelector(".settings-tab.active");
  const heading = layer.querySelector(".settings-nav-title");
  title.textContent = view === "page" && active ? active.textContent : heading ? heading.textContent : layer.getAttribute("aria-label");
}

function showSettingsMenuOnPhone(id) {
  if (isPhoneWidth(window.innerWidth) && !$(id).classList.contains("hidden")) settingsPhoneView($(id), "menu");
}

function onPhoneHistory(e) {
  const entry = e.state || {};
  const layer = document.querySelector(".settings-layer:not(.hidden)");
  if (layer && !entry.settings) closeSettingsLayers();
  else if (layer && !entry.page) settingsPhoneView(layer, "menu");
  setPhonePane(entry.pane || "list", { fromHistory: true });
}

function wireSettingsPhoneBars() {
  for (const layer of document.querySelectorAll(".settings-layer")) {
    const bar = el("div", "settings-phone-bar");
    const back = el("button", "icon-btn settings-phone-back");
    back.title = "Back";
    back.setAttribute("aria-label", "Back");
    back.appendChild(svgIcon("chevronLeft", 22));
    back.onclick = () => {
      if (history.state && history.state.page) history.back();
      else settingsPhoneView(layer, "menu");
    };
    const close = el("button", "icon-btn settings-phone-close");
    close.title = "Close";
    close.setAttribute("aria-label", "Close");
    close.appendChild(svgIcon("close", 20));
    close.onclick = () => {
      const depth = history.state && history.state.settings ? (history.state.page ? 2 : 1) : 0;
      layer.querySelector(".settings-close").click();
      if (depth) history.go(-depth);
    };
    bar.append(back, el("span", "settings-phone-title"), close);
    layer.prepend(bar);
    layer.querySelector(".settings-nav").addEventListener("click", e => {
      const tab = e.target.closest(".settings-tab[data-tab]");
      if (tab && isPhoneWidth(window.innerWidth)) history.pushState({ pane: phonePane(), settings: true, page: true }, "");
    }, true);
  }
}

function wirePhoneLayout() {
  $("app").dataset.pane = "list";
  window.addEventListener("popstate", onPhoneHistory);
  wireSettingsPhoneBars();
  $("phone-back").onclick = () => phoneStep("back");
  $("channel-list").addEventListener("click", e => {
    const row = e.target.closest(".channel");
    if (row && rowOpensConversation([...row.classList], Boolean(e.target.closest("button")))) phoneStep("open");
  });
}

async function markMobileApp() {
  const mobile = await invoke("is_mobile_app").catch(() => false);
  document.body.classList.toggle("mobile-app", Boolean(mobile));
}

function setViewMode(mode) {
  closeDiscover();
  state.view = mode;
  const dm = mode === "dm";
  const group = mode === "server" && Boolean(state.currentServer && state.currentServer.group);
  const home = dm || group;
  $("dm-call-btn").classList.toggle("hidden", !home);
  $("group-add-btn").classList.toggle("hidden", !home);
  $("group-leave-btn").classList.toggle("hidden", !group);
  paintNarrowRail();
  $("members-btn").classList.toggle("hidden", dm);
  $("member-list").classList.toggle("hidden", dm);
  $("invite-btn").classList.toggle("hidden", home);
  $("server-settings-btn").classList.toggle("hidden", home);
  $("server-name").classList.toggle("hidden", home);
  if (home) paintSidebarBanner();
  $("dm-search").classList.toggle("hidden", !home);
  syncWindowTitle();
  syncAttachAccess();
  $("home-btn").classList.toggle("active", home);
  $("app").classList.toggle("dm-mode", home);
  if (dm) $("cam-grid").classList.add("hidden");
  if (dm) syncAwaitedBar();
  if (!dm) {
    $("chat-hash").textContent = "#";
    $("chat-presence").classList.add("hidden");
    $("dm-profile").classList.add("hidden");
    $("dm-profile-btn").classList.add("hidden");
    $("dm-search").value = "";
  }
  renderShareButton();
  syncPinButton();
  syncSearchBox();
  syncRulesGate();
}

async function selectServer(serverId) {
  closeContextMenus();
  const previous = state.currentServer && state.currentServer.id;
  try {
    state.currentServer = await invoke("get_server", { serverId });
  } catch (e) {
    toast(String(e));
    return;
  }
  if (previous !== serverId) resetSearch();
  state.historyAnchor = null;
  setViewMode("server");
  state.currentDm = null;
  if (state.currentServer.group) rememberSwitcherRecent(`group:${serverId}`);
  if (state.voiceServer && state.voiceServer.id === serverId) state.voiceServer.activities_enabled = Boolean(state.currentServer.activities_enabled);
  const firstText = state.currentServer.channels.find(c => c.kind === "text" && c.can_view !== false);
  renderServerRail();
  renderMembers();
  await selectChannel(firstText || null);
  if (state.currentServer.group) paintGroupHeader();
}

function groupVoiceChannel(server) {
  return (server.channels || []).find(c => c.kind === "voice") || null;
}

function paintGroupHeader() {
  const server = state.currentServer;
  const count = server.members.length;
  $("chat-hash").replaceChildren(svgIcon("people", 22));
  $("channel-name").textContent = server.name;
  $("channel-topic").textContent = `${count} ${count === 1 ? "member" : "members"}`;
  $("message-input").placeholder = `Message ${server.name}`;
  $("group-add-btn").title = "Add People";
  $("group-leave-btn").title = "Leave Group";
  renderCallButton();
}

function toggleGroupCall() {
  const channel = groupVoiceChannel(state.currentServer);
  if (!channel) return;
  if (channel.user_limit && channel.voice_users.length >= channel.user_limit && state.voiceChannelId !== channel.id) {
    toast("That call is full");
    return;
  }
  toggleVoice(channel);
}

let groupModal = { server: null, picked: new Set() };

function openGroupModal(serverId, preselect = []) {
  groupModal = { server: serverId, picked: new Set(preselect) };
  $("group-modal-title").textContent = serverId ? "Add People" : "New Group";
  $("group-modal-sub").textContent = serverId
    ? "They join this group and see its messages from now on."
    : "Pick the people for this group. Everyone in it can message, call and add others.";
  $("group-go").textContent = serverId ? "Add" : "Create Group";
  $("group-search").value = "";
  renderGroupList();
  showModal("modal-group");
  $("group-search").focus();
}

function groupMemberIds(serverId) {
  const server = state.currentServer && state.currentServer.id === serverId ? state.currentServer : null;
  return new Set(server ? server.members.map(m => m.id) : []);
}

function renderGroupList() {
  const list = $("group-list");
  list.innerHTML = "";
  const inside = groupMemberIds(groupModal.server);
  const query = $("group-search").value.trim().toLowerCase();
  const people = [...state.people.values()]
    .filter(p => !query || p.name.toLowerCase().includes(query))
    .sort((a, b) => (a.presence === "offline") - (b.presence === "offline") || a.name.localeCompare(b.name));
  for (const person of people) {
    const already = inside.has(person.id);
    const picked = groupModal.picked.has(person.id);
    const row = el("button", "person-row" + (picked ? " picked" : "") + (already ? " in-group" : ""));
    row.appendChild(avatarWithPresence(person.name, person.id));
    const meta = el("div", "dm-meta");
    meta.appendChild(el("div", "dm-name", person.name));
    meta.appendChild(el("div", "dm-preview", already ? "Already in this group" : person.servers.map(s => s.name).join(", ")));
    row.appendChild(meta);
    const pick = el("span", "pick");
    pick.appendChild(svgIcon("check", 14));
    row.appendChild(pick);
    row.disabled = already;
    row.onclick = () => {
      if (picked) groupModal.picked.delete(person.id);
      else groupModal.picked.add(person.id);
      renderGroupList();
    };
    list.appendChild(row);
  }
  if (!people.length) {
    list.appendChild(el("div", "dm-list-empty", state.people.size ? "Nobody matches that name." : "Join or create a server first. You can add anyone you share a server with."));
  }
  const n = groupModal.picked.size;
  $("group-go").disabled = n === 0;
  $("group-go").textContent = groupModal.server ? (n > 1 ? `Add ${n}` : "Add") : "Create Group";
}

async function confirmGroupModal() {
  const ids = [...groupModal.picked];
  if (!ids.length) return;
  try {
    if (groupModal.server) {
      await invoke("add_group_members", { serverId: groupModal.server, userIds: ids });
      hideModals();
      await refreshServers();
      await selectServer(groupModal.server);
      toast(ids.length === 1 ? "Added to the group" : `Added ${ids.length} people to the group`);
    } else {
      const summary = await invoke("create_group_dm", { userIds: ids });
      hideModals();
      await refreshServers();
      await refreshPeople();
      await selectServer(summary.id);
    }
  } catch (e) {
    toast(String(e));
  }
}

async function refreshPeople() {
  const views = [];
  for (const summary of state.servers) {
    try {
      views.push(await invoke("get_server", { serverId: summary.id }));
    } catch {
      continue;
    }
  }
  for (const server of views) rememberAvatars(server.members);
  friendPresenceViews.clear();
  for (const server of views) noteFriendPresence(server);
  state.people = indexPeople(views, state.user ? state.user.id : null);
}

function personPresence(userId) {
  const person = state.people.get(userId);
  if (!person || person.presence === "invisible") return "offline";
  return person.presence;
}

function avatarWithPresence(name, userId, className = "avatar") {
  const avatar = makeAvatar(userId, name, className);
  avatar.appendChild(el("span", "presence-dot " + personPresence(userId)));
  return avatar;
}

function formatShortTime(iso) {
  if (!iso) return "";
  const date = new Date(iso);
  const now = new Date();
  if (date.toDateString() === now.toDateString()) {
    return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  }
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (date.toDateString() === yesterday.toDateString()) return "Yesterday";
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

async function enterDmMode() {
  setViewMode("dm");
  state.currentServer = null;
  state.currentChannel = null;
  renderServerRail();
  await refreshPeople();
  const threads = await renderDmThreads();
  if (state.currentDm) {
    await selectDm(state.currentDm.user, state.currentDm.name);
  } else if (threads.length && threads[0].kind === "group") {
    await selectServer(threads[0].user);
  } else if (threads.length) {
    await selectDm(threads[0].user, threads[0].name);
  } else {
    renderDmHome();
  }
}

function renderDmHome() {
  $("chat-hash").textContent = "@";
  $("channel-name").textContent = "Direct Messages";
  $("channel-topic").textContent = "";
  $("chat-presence").classList.add("hidden");
  $("dm-profile").classList.add("hidden");
  $("dm-profile-btn").classList.add("hidden");
  $("message-input").placeholder = "Pick a conversation to start chatting";
  const box = $("messages");
  box.innerHTML = "";
  if (!railServers().length) {
    box.appendChild(buildWelcomeHome());
    return;
  }
  const empty = el("div", "empty-state dm-empty");
  const art = el("div", "dm-empty-art");
  art.appendChild(svgIcon("chat", 56));
  empty.appendChild(art);
  empty.appendChild(el("h3", null, "No direct messages yet"));
  empty.appendChild(el("p", null, "Private chats go straight to the other person over P2P. Nothing is stored on a server."));
  const start = el("button", "btn-primary", "Start a Conversation");
  start.onclick = openNewDm;
  empty.appendChild(start);
  box.appendChild(empty);
}

function buildWelcomeHome() {
  const home = el("div", "empty-state welcome-home");
  const mark = el("img", "welcome-mark");
  mark.src = "images/commy_v3_noback.png";
  mark.alt = "";
  mark.draggable = false;
  home.appendChild(mark);
  const name = state.user ? state.user.username : "";
  home.appendChild(el("h3", null, name ? `Welcome, ${name}` : "Welcome to Commy"));
  home.appendChild(el("p", null, "Servers are where your friends gather: text channels, voice and screen sharing, all connected directly between members. Start one, or join a friend's with an invite."));
  const actions = el("div", "welcome-actions");
  const create = el("button", "btn-primary", "Create a Server");
  create.onclick = () => { showModal("modal-add-server"); $("create-name-input").focus(); };
  const join = el("button", "btn-secondary", "Join with an Invite");
  join.onclick = () => { showModal("modal-add-server"); $("join-ticket-input").focus(); };
  actions.append(create, join);
  home.appendChild(actions);
  return home;
}

async function renderDmThreads() {
  const threads = await invoke("list_dm_threads");
  const list = $("channel-list");
  list.innerHTML = "";
  state.dmThreads = threads;
  list.appendChild(savedSidebarRow());
  const query = $("dm-search").value.trim().toLowerCase();
  const matches = threads.filter(t => !query || t.name.toLowerCase().includes(query));
  const shown = matches.filter(t => t.kind === "inbox" || t.kind === "group");
  const requests = matches.filter(t => t.kind !== "inbox" && t.kind !== "group");

  const group = el("div", "channel-group dm-group");
  group.appendChild(el("span", null, "Direct Messages"));
  const add = el("button", "dm-group-add");
  add.title = "New Message";
  add.appendChild(svgIcon("plus", 16));
  add.onclick = openNewDm;
  group.appendChild(add);
  list.appendChild(group);

  if (!threads.length) {
    list.appendChild(el("div", "dm-list-empty", "Your conversations will show up here."));
  } else if (!matches.length) {
    list.appendChild(el("div", "dm-list-empty", "No conversations match."));
  }
  const split = splitPinned(shown, listPins.dms, t => t.user);
  if (split.pinned.length) {
    list.appendChild(el("div", "channel-group dm-group", "Pinned"));
    for (const thread of split.pinned) list.appendChild(dmThreadRow(thread, true));
  }
  const friendsShown = !query && (friends.friends.length || friends.incoming.length);
  if (split.pinned.length && !friendsShown && (split.rest.length || requests.length)) {
    list.appendChild(el("div", "channel-group dm-group", "Direct Messages"));
  }
  if (friendsShown) {
    list.appendChild(el("div", "channel-group dm-group", `Friends - ${friends.friends.length}`));
    for (const req of friends.incoming) {
      const row = el("div", "channel dm-thread friend-request");
      row.appendChild(avatarWithPresence(req.name, req.id));
      const meta = el("div", "dm-meta");
      meta.appendChild(el("div", "dm-name", req.name));
      const line = el("div", "dm-preview friend-note", friendRequestLine(req));
      line.title = req.note || "";
      meta.appendChild(line);
      row.appendChild(meta);
      const yes = el("button", "icon-btn friend-yes");
      yes.title = "Accept";
      yes.appendChild(svgIcon("check", 16));
      yes.onclick = e => {
        e.stopPropagation();
        friendAction(req.id, "accept", req.name);
      };
      const no = el("button", "icon-btn friend-no");
      no.title = "Decline";
      no.appendChild(svgIcon("close", 16));
      no.onclick = e => {
        e.stopPropagation();
        friendAction(req.id, "decline", req.name);
      };
      row.append(yes, no);
      list.appendChild(row);
    }
    const order = { online: 0, idle: 1, do_not_disturb: 2 };
    const sorted = [...friends.friends].sort((a, b) => (order[personPresence(a.id)] ?? 3) - (order[personPresence(b.id)] ?? 3));
    for (const friend of sorted) {
      const row = el("div", "channel dm-thread friend-row");
      row.appendChild(avatarWithPresence(friend.name, friend.id));
      const meta = el("div", "dm-meta");
      meta.appendChild(el("div", "dm-name", friend.name));
      const person = state.people.get(friend.id);
      meta.appendChild(appendEmojiText(el("div", "dm-preview"), (person && statusLine(person)) || PRESENCE_LABELS[personPresence(friend.id)] || "Offline"));
      row.appendChild(meta);
      row.onclick = () => messageUser(friend.id, friend.name);
      row.oncontextmenu = e => {
        e.preventDefault();
        openMemberMenu({ id: friend.id, name: friend.name }, e.clientX, e.clientY, { dm: true });
      };
      list.appendChild(row);
    }
    list.appendChild(el("div", "channel-group dm-group", "Direct Messages"));
  }
  const rows = [...split.rest];
  if (requests.length) {
    rows.push({ header: `Message Requests - ${requests.length}` });
    rows.push(...requests.filter(t => t.kind === "request"));
    const spam = requests.filter(t => t.kind === "spam");
    if (spam.length) rows.push({ header: `Spam - ${spam.length}` }, ...spam);
  }
  for (const thread of rows) {
    if (thread.header) {
      list.appendChild(el("div", "channel-group dm-group dm-requests-head", thread.header));
      continue;
    }
    list.appendChild(dmThreadRow(thread, false));
  }
  if (query) {
    const known = new Set(threads.map(t => t.user));
    const extra = [...state.people.values()]
      .filter(p => !known.has(p.id) && p.name.toLowerCase().includes(query))
      .slice(0, 8);
    if (extra.length) {
      list.appendChild(el("div", "channel-group", "Start a Conversation"));
      for (const person of extra) {
        const row = el("div", "channel dm-thread");
        row.appendChild(avatarWithPresence(person.name, person.id));
        const meta = el("div", "dm-meta");
        meta.appendChild(el("div", "dm-name", person.name));
        meta.appendChild(el("div", "dm-preview", person.servers.map(s => s.name).join(", ")));
        row.appendChild(meta);
        row.onclick = () => messageUser(person.id, person.name);
        list.appendChild(row);
      }
    }
  }
  return threads;
}

function dmThreadRow(thread, pinned) {
  if (thread.kind === "group") return groupThreadRow(thread, pinned);
  const row = el("div", "channel dm-thread");
  if (state.currentDm && thread.user === state.currentDm.user) row.classList.add("active");
  if (unread.dms.get(thread.user)) row.classList.add("unread");
  row.appendChild(avatarWithPresence(thread.name, thread.user));
  const meta = el("div", "dm-meta");
  const top = el("div", "dm-top");
  top.appendChild(el("span", "dm-name", thread.name));
  if (pinned) top.appendChild(pinMark("dm-pin"));
  top.appendChild(el("span", "dm-time", formatShortTime(thread.last_at)));
  meta.appendChild(top);
  meta.appendChild(el("div", "dm-preview", thread.kind === "spam" ? "Hidden as possible spam" : plainText(thread.last_message) || "No messages yet"));
  if (thread.kind !== "inbox") row.classList.add("dm-request");
  row.appendChild(meta);
  row.onclick = () => selectDm(thread.user, thread.name);
  row.oncontextmenu = e => {
    e.preventDefault();
    openMemberMenu({ id: thread.user, name: thread.name }, e.clientX, e.clientY, { dm: true });
  };
  return row;
}

function groupAvatar(className = "avatar group-avatar") {
  const node = el("div", className);
  node.appendChild(svgIcon("people", 18));
  return node;
}

function groupThreadRow(thread, pinned = false) {
  const row = el("div", "channel dm-thread");
  if (isGroupView() && thread.user === state.currentServer.id) row.classList.add("active");
  if (serverUnreadCount(thread.user)) row.classList.add("unread");
  row.appendChild(groupAvatar());
  const meta = el("div", "dm-meta");
  const top = el("div", "dm-top");
  top.appendChild(el("span", "dm-name", thread.name));
  if (pinned) top.appendChild(pinMark("dm-pin"));
  top.appendChild(el("span", "dm-time", formatShortTime(thread.last_at)));
  meta.appendChild(top);
  meta.appendChild(el("div", "dm-preview", plainText(thread.last_message) || "No messages yet"));
  row.appendChild(meta);
  row.onclick = () => selectServer(thread.user);
  row.oncontextmenu = e => {
    e.preventDefault();
    openGroupMenu(thread, e.clientX, e.clientY);
  };
  return row;
}

function openGroupMenu(thread, x, y) {
  closeFloating();
  const menu = $("member-menu");
  menu.replaceChildren(el("div", "ctx-title", thread.name));
  const { item, sep } = contextMenuBuilder(menu);
  item("Open", () => selectServer(thread.user));
  const pinned = dmIsPinned(thread.user);
  item(pinned ? "Unpin" : "Pin", () => setDmPinned(thread.user, !pinned));
  item("Add People", async () => {
    if (!isGroupView() || state.currentServer.id !== thread.user) await selectServer(thread.user);
    openGroupModal(thread.user);
  });
  sep();
  item("Leave Group", () => openLeaveServer(thread.user), "danger");
  placeMenuAt(menu, x, y);
}

async function selectDm(userId, name) {
  if (!state.currentDm || state.currentDm.user !== userId) cancelReply();
  state.currentDm = { user: userId, name };
  rememberSwitcherRecent(`dm:${userId}`);
  clearDmUnread(userId);
  if (state.view !== "dm") setViewMode("dm");
  $("chat-hash").textContent = "@";
  $("channel-name").textContent = name;
  renderDmHeader(userId, name);
  $("message-input").placeholder = `Message @${name}`;
  $("dm-profile-btn").classList.remove("hidden");
  renderCallButton();
  const threads = await renderDmThreads();
  renderDmRequestBar(threads.find(t => t.user === userId) || null);
  const messages = await invoke("get_dm_messages", { userId });
  renderMessages(messages.map(dmToRenderable));
  renderTypingStrip();
}

function renderCallButton() {
  const btn = $("dm-call-btn");
  if (isGroupView()) {
    const channel = groupVoiceChannel(state.currentServer);
    const joined = Boolean(channel) && state.voiceChannelId === channel.id && state.voiceServer && state.voiceServer.id === state.currentServer.id;
    btn.classList.toggle("hidden", !channel);
    btn.replaceChildren(svgIcon(joined ? "hangup" : "phone", 20));
    btn.title = joined ? "Leave Call" : "Start Group Call";
    btn.classList.toggle("active", joined);
    return;
  }
  const dm = inDmView();
  btn.classList.toggle("hidden", !dm);
  if (!dm) return;
  $("group-add-btn").title = "Start a Group";
  const talking = state.call && state.call.user === state.currentDm.user;
  btn.replaceChildren(svgIcon(talking ? "hangup" : "phone", 20));
  btn.title = talking ? "End Call" : "Start Voice Call";
  btn.classList.toggle("active", Boolean(talking));
}

let callRing = null;

function stopCallRing() {
  if (callRing) clearInterval(callRing);
  callRing = null;
}

function callPanel(view) {
  state.voiceChannelId = view ? "call" : null;
  state.voiceServer = view ? { id: null, call: view.user, name: view.name, channelName: "Call" } : null;
  renderVoicePanel();
  renderCallButton();
}

async function startDmCall() {
  if (!inDmView()) return;
  if (state.call && state.call.user === state.currentDm.user) return endDmCall();
  try {
    onDmCall(await invoke("start_dm_call", { userId: state.currentDm.user }));
  } catch (e) {
    toast(String(e));
  }
}

async function answerDmCall(accept) {
  try {
    await invoke("answer_dm_call", { accept });
    if (!accept) onDmCall({ ...(state.call || {}), state: "ended", reason: "declined" });
  } catch (e) {
    toast(String(e));
  }
}

async function endDmCall() {
  try {
    await invoke("end_dm_call");
  } catch (e) {
    toast(String(e));
  }
}

function callEndedLine(view) {
  const name = view.name || "They";
  switch (view.reason) {
    case "declined": return `${name} declined the call`;
    case "unanswered": return `${name} did not answer`;
    case "missed": return `Missed call from ${name}`;
    case "elsewhere": return "Answered on another device";
    case "failed": return "The call could not connect";
    default: return "Call ended";
  }
}

function onDmCall(view) {
  const wasActive = state.call && state.call.state === "active";
  if (view.state === "ended") {
    state.call = null;
    stopCallRing();
    if (!$("modal-call").classList.contains("hidden")) hideModals();
    clearSpeaking();
    callPanel(null);
    toast(callEndedLine(view));
    if (wasActive) playSound("leave");
    return;
  }
  state.call = view;
  if (view.state === "ringing" && !view.outgoing) {
    const avatar = $("call-avatar");
    avatar.replaceChildren(makeAvatar(view.user, view.name, "avatar avatar-xl"));
    $("call-name").textContent = view.name;
    showModal("modal-call");
    stopCallRing();
    playSound("dm");
    callRing = setInterval(() => playSound("dm"), 3000);
    return;
  }
  if (view.state === "active") {
    stopCallRing();
    if (!$("modal-call").classList.contains("hidden")) hideModals();
    if (!wasActive) playSound("join");
  }
  callPanel(view);
}

function renderDmHeader(userId, name) {
  const presence = personPresence(userId);
  const person = state.people.get(userId);
  $("chat-presence").className = `presence-dot inline ${presence}`;
  $("chat-presence").title = presenceLabel({ ...(person || {}), presence });
  setEmojiText($("channel-topic"), (person && subLine(person)) || "");
  renderDmProfile(userId, name);
}

function renderDmProfile(userId, name) {
  const person = state.people.get(userId);
  const presence = personPresence(userId);
  $("dm-profile").classList.toggle("hidden", !state.showDmProfile);
  $("dm-profile-btn").classList.toggle("active", state.showDmProfile);
  $("dm-profile-btn").title = state.showDmProfile ? "Hide User Profile" : "Show User Profile";
  paintAvatar($("dm-profile-avatar"), userId, name);
  $("dm-profile-presence").className = `presence-dot ${presence}`;
  $("dm-profile-banner").style.background = bannerColor(userId);
  $("dm-profile-name").textContent = name;
  const seen = { ...(person || {}), presence };
  setEmojiText($("dm-profile-status"), (person && (isShowingActivity(seen) ? statusBesideGame(seen) : subLine(seen))) || presenceLabel(seen));
  showActivityIn($("dm-profile-activity"), person && seen);
  const servers = person ? person.servers : [];
  $("dm-profile-mutual-label").textContent = `Mutual Servers - ${servers.length}`;
  const list = $("dm-profile-servers");
  list.innerHTML = "";
  for (const server of servers) {
    const row = el("button", "dm-profile-server");
    row.appendChild(el("span", "server-chip", initials(server.name)));
    row.appendChild(el("span", null, server.name));
    row.onclick = () => selectServer(server.id).then(phoneShowChat);
    list.appendChild(row);
  }
  if (!servers.length) list.appendChild(el("div", "dm-preview", "No servers in common right now"));
  $("dm-profile-block").onclick = () => openBlock(userId, name);
}

function bannerColor(userId) {
  return starfield(userId).css;
}

function roleGradient(hex) {
  const n = parseInt(String(hex).replace(/^#/, ""), 16);
  if (Number.isNaN(n)) return bannerColor(hex);
  const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  const max = Math.max(r, g, b) / 255, min = Math.min(r, g, b) / 255;
  const l = (max + min) / 2;
  const d = max - min;
  let h = 0;
  if (d) {
    const rr = r / 255, gg = g / 255, bb = b / 255;
    h = max === rr ? (gg - bb) / d + (gg < bb ? 6 : 0) : max === gg ? (bb - rr) / d + 2 : (rr - gg) / d + 4;
    h *= 60;
  }
  const sat = d ? Math.round((d / (1 - Math.abs(2 * l - 1))) * 100) : 0;
  return `linear-gradient(135deg, hsl(${Math.round(h)} ${Math.min(sat, 70)}% ${Math.round(l * 100)}%), hsl(${Math.round((h + 35) % 360)} ${Math.min(sat, 60)}% ${Math.max(18, Math.round(l * 100) - 24)}%))`;
}

function buildDmIntro() {
  const { user, name } = state.currentDm;
  const person = state.people.get(user);
  const intro = el("div", "dm-intro");
  intro.appendChild(makeAvatar(user, name, "avatar avatar-xl"));
  intro.appendChild(el("h2", null, name));
  const line = el("p");
  line.appendChild(document.createTextNode("This is the beginning of your direct message history with "));
  line.appendChild(el("strong", null, name));
  line.appendChild(document.createTextNode("."));
  intro.appendChild(line);
  if (person && person.servers.length) {
    const n = person.servers.length;
    intro.appendChild(el("div", "dm-intro-mutual", `${n} mutual server${n === 1 ? "" : "s"}`));
  }
  return intro;
}

function renderInvitePeople() {
  const list = $("invite-people");
  list.innerHTML = "";
  const server = state.currentServer;
  if (!server) return;
  const query = $("invite-friend-search").value;
  const people = inviteCandidates(state.people, friends.friends.map(f => f.id), server.members.map(m => m.id), query).slice(0, 40);
  for (const person of people) {
    const row = el("div", "person-row invite-person");
    row.appendChild(avatarWithPresence(person.name, person.id));
    const meta = el("div", "dm-meta");
    meta.appendChild(el("div", "dm-name", person.name));
    meta.appendChild(el("div", "dm-preview", person.friend ? "Friend" : person.servers.map(s => s.name).join(", ")));
    row.appendChild(meta);
    const sent = state.invitedNow.has(person.id);
    const btn = el("button", sent ? "btn-secondary btn-small" : "btn-primary btn-small", sent ? "Sent" : "Invite");
    btn.disabled = sent;
    btn.onclick = async () => {
      btn.disabled = true;
      try {
        await invoke("open_dm", { userId: person.id });
        const sentDm = await invoke("send_dm", { userId: person.id, content: `Come join me in ${server.name}! ${$("invite-text").value}` });
        state.invitedNow.add(person.id);
        btn.className = "btn-secondary btn-small";
        btn.textContent = "Sent";
        dmSentToast(sentDm, person.name, `Invited ${person.name} to ${server.name}`);
      } catch (e) {
        btn.disabled = false;
        toast(String(e));
      }
    };
    row.appendChild(btn);
    list.appendChild(row);
  }
  if (!people.length) list.appendChild(el("div", "dm-list-empty", query.trim() ? "Nobody matches that name." : "Everyone you know is already here."));
}

function openNewDm() {
  $("new-dm-search").value = "";
  renderNewDmList();
  showModal("modal-new-dm");
  $("new-dm-search").focus();
}

function renderNewDmList() {
  const list = $("new-dm-list");
  list.innerHTML = "";
  const query = $("new-dm-search").value.trim().toLowerCase();
  if (!query && state.people.size) {
    const row = el("button", "person-row new-group");
    row.appendChild(groupAvatar());
    const meta = el("div", "dm-meta");
    meta.appendChild(el("div", "dm-name", "New Group"));
    meta.appendChild(el("div", "dm-preview", "A private chat for a few friends"));
    row.appendChild(meta);
    row.onclick = () => {
      hideModals();
      openGroupModal(null);
    };
    list.appendChild(row);
  }
  const people = [...state.people.values()]
    .filter(p => !query || p.name.toLowerCase().includes(query))
    .sort((a, b) => (a.presence === "offline") - (b.presence === "offline") || a.name.localeCompare(b.name));
  for (const person of people) {
    const row = el("button", "person-row");
    row.appendChild(avatarWithPresence(person.name, person.id));
    const meta = el("div", "dm-meta");
    meta.appendChild(el("div", "dm-name", person.name));
    meta.appendChild(el("div", "dm-preview", person.servers.map(s => s.name).join(", ")));
    row.appendChild(meta);
    row.onclick = () => {
      hideModals();
      messageUser(person.id, person.name);
    };
    list.appendChild(row);
  }
  if (!people.length) {
    list.appendChild(el("div", "dm-list-empty", state.people.size
      ? "Nobody matches that name."
      : "Join or create a server first. You can message anyone you share a server with."));
  }
}

function dmToRenderable(m) {
  return {
    id: m.id,
    author_id: m.from,
    author_name: m.from_name,
    content: m.content,
    created_at: m.created_at,
    edited: Boolean(m.edited),
    reply_to: m.reply_to || null,
    reply_preview: m.reply_preview || null,
    reactions: m.reactions || [],
    attachments: m.attachments || [],
    pending: Boolean(m.pending),
    voice_invite: m.voice_invite || null,
  };
}

function dmSentToast(sent, name, done) {
  toast(sent && sent.pending ? `Will send to ${name} when they come online` : done);
}

async function messageUser(userId, name) {
  if (state.user && userId === state.user.id) return;
  try {
    const thread = await invoke("open_dm", { userId });
    state.currentDm = { user: thread.user, name: thread.name };
    await enterDmMode();
    phoneShowChat();
  } catch (e) {
    toast(String(e));
  }
}

async function sendDm() {
  const input = $("message-input");
  const content = input.value.trim();
  if (!content || !state.currentDm) return;
  input.value = "";
  fitComposer(input);
  const replyTo = state.replyingTo ? state.replyingTo.id : null;
  cancelReply();
  try {
    await invoke("send_dm", { userId: state.currentDm.user, content: encodeCustomEmojis(content), replyTo });
    await selectDm(state.currentDm.user, state.currentDm.name);
  } catch (e) {
    input.value = content;
    fitComposer(input);
    toast(String(e));
  }
}

async function selectChannel(channel) {
  closeContextMenus();
  const switched = Boolean(channel) && (!state.currentChannel || channel.id !== state.currentChannel.id);
  if (state.currentChannel && (!channel || channel.id !== state.currentChannel.id)) state.previousChannel = state.currentChannel;
  state.currentChannel = channel;
  if (switched && state.currentServer) await invoke("open_channel", { serverId: state.currentServer.id, channelId: channel.id }).catch(() => {});
  if (channel && state.currentServer && !state.currentServer.group) rememberSwitcherRecent(`channel:${channel.id}`);
  if (channel) openChannelWithLine(channel);
  renderDmRequestBar(null);
  syncRulesGate();
  syncAwaitedBar();
  if (channel) clearChannelUnread(channel.id);
  renderChannels();
  $("channel-name").replaceChildren(channel ? channelNameNode(channel) : document.createTextNode("no channel"));
  $("chat-hash").replaceChildren(channelHeaderMark(channel));
  $("channel-topic").textContent = (channel && channel.topic) || "";
  $("message-input").placeholder = channel ? `Message #${channelPlainName(channel)}` : "Select a channel";
  syncPinButton();
  syncSlowMode();
  renderTypingStrip();
  if (!channel || ageGated(channel)) return renderMessages([]);
  const messages = await fetchChannelMessages(channel);
  renderChannelLog(messages);
  if (state.currentChannel === channel && !state.historyAnchor) advanceRead(channel.id, messages);
}

async function sendMessage() {
  const input = $("message-input");
  const gif = input.value.trim().match(/^\/gif(?:\s+(.*))?$/i);
  if (gif) {
    input.value = "";
  fitComposer(input);
    openGifPicker(gif[1] || "");
    return;
  }
  if (state.view === "dm") return sendDm();
  const typed = input.value.trim();
  const { tts, content } = parseTtsPrefix(typed);
  if (!content || !state.currentServer || !state.currentChannel) return;
  if (tts && !canSendTtsHere()) {
    toast("You can't send text-to-speech messages in this server");
    return;
  }
  stopSpeaking();
  let command = null;
  const slash = matchSlashCommand(content, slashCatalogHere());
  if (slash) {
    const { members, channels } = state.currentServer;
    const parsed = parseSlashOptions(slash.entry, slash.rest, { members, channels });
    if (parsed.problem) return toast(parsed.problem);
    command = { bot: slash.entry.bot, command: slash.entry.name, options: parsed.options };
  }
  input.value = "";
  fitComposer(input);
  const replyTo = state.replyingTo ? state.replyingTo.id : null;
  typingSender.sent();
  $("messages").dataset.view = "";
  state.historyAnchor = null;
  readState.line = null;
  readState.held = null;
  cancelReply();
  try {
    const previewUrl = command ? null : await previewUrlFor(content);
    clearLinkDraft();
    const outgoing = command ? content : encodeOutgoing(content);
    await invoke("send_message", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      content: outgoing,
      replyTo,
      previewUrl,
      tts,
      command,
    });
    pushOverlayChat(state.user.id, state.user.display_name || state.user.username, outgoing, false);
    noteSlowSend(state.currentChannel.id, 0);
    await selectChannel(state.currentChannel);
  } catch (e) {
    input.value = typed;
    fitComposer(input);
    const wait = slowModeWaitFromError(e);
    if (wait) noteSlowSend(state.currentChannel.id, wait);
    toast(String(e));
  }
}

const LINK_PREVIEW_KEY = "commy.linkPreviews";
const SEND_PREVIEW_WAIT_MS = 2500;
const LINK_DRAFT_DELAY_MS = 350;
const linkDraft = { url: null, dismissed: null, preview: null, loading: null, token: 0, timer: null };

function linkPreviewPrefs() {
  try {
    return readLinkPreviewPrefs(localStorage.getItem(LINK_PREVIEW_KEY));
  } catch {
    return readLinkPreviewPrefs(null);
  }
}

function saveLinkPreviewPrefs(prefs) {
  try { localStorage.setItem(LINK_PREVIEW_KEY, JSON.stringify(prefs)); } catch {}
}

function renderLinkPreviewSettings() {
  const prefs = linkPreviewPrefs();
  $("link-preview-send-toggle").checked = prefs.send;
  $("link-preview-show-toggle").checked = prefs.show;
}

function onLinkPreviewSettingsChange() {
  saveLinkPreviewPrefs({ send: $("link-preview-send-toggle").checked, show: $("link-preview-show-toggle").checked });
  clearLinkDraft();
  if (state.view === "server" && state.currentChannel) selectChannel(state.currentChannel);
}

function previewableDraftLink(text) {
  if (state.view !== "server" || !linkPreviewPrefs().send) return null;
  return previewableLink(text, url => imageLinks(url).length > 0 || Boolean(findInviteLink(url)));
}

function scheduleLinkDraft(delay = LINK_DRAFT_DELAY_MS) {
  clearTimeout(linkDraft.timer);
  linkDraft.timer = setTimeout(refreshLinkDraft, delay);
}

function refreshLinkDraft() {
  const url = previewableDraftLink($("message-input").value);
  if (url === linkDraft.url) return;
  linkDraft.url = url;
  linkDraft.preview = null;
  linkDraft.loading = null;
  const token = ++linkDraft.token;
  renderLinkDraft();
  if (!url || url === linkDraft.dismissed) return;
  if (!canEmbedLinks(state.currentServer)) {
    linkDraft.preview = { off: true, url };
    renderLinkDraft();
    return;
  }
  linkDraft.loading = invoke("prepare_link_preview", { url })
    .catch(() => null)
    .then(preview => {
      if (token === linkDraft.token) {
        linkDraft.preview = preview;
        renderLinkDraft();
      }
      return preview;
    });
}

function clearLinkDraft() {
  clearTimeout(linkDraft.timer);
  linkDraft.token++;
  Object.assign(linkDraft, { url: null, dismissed: null, preview: null, loading: null });
  renderLinkDraft();
}

function renderLinkDraft() {
  const bar = $("link-draft");
  const preview = linkDraft.preview;
  const visible = Boolean(preview) && linkDraft.url !== linkDraft.dismissed && state.view === "server";
  bar.classList.toggle("hidden", !visible);
  if (!visible) return;
  const thumb = $("link-draft-thumb");
  thumb.replaceChildren();
  bar.classList.toggle("off", Boolean(preview.off));
  if (preview.off) {
    thumb.appendChild(svgIcon("link", 18));
    $("link-draft-title").textContent = LINK_EMBED_OFF.title;
    $("link-draft-site").textContent = LINK_EMBED_OFF.sub;
    return;
  }
  if (preview.image) {
    const img = el("img");
    img.alt = "";
    img.src = preview.image;
    thumb.appendChild(img);
  } else {
    thumb.appendChild(svgIcon("link", 18));
  }
  $("link-draft-title").textContent = preview.title || preview.description || preview.url;
  $("link-draft-site").textContent = previewSite(preview);
}

function dismissLinkDraft() {
  linkDraft.dismissed = linkDraft.url;
  renderLinkDraft();
}

async function previewUrlFor(content) {
  const url = previewableDraftLink(content);
  if (!url || url === linkDraft.dismissed || !canEmbedLinks(state.currentServer)) return null;
  const pending = url === linkDraft.url && linkDraft.loading
    ? linkDraft.loading
    : invoke("prepare_link_preview", { url }).catch(() => null);
  const preview = await Promise.race([pending, new Promise(resolve => setTimeout(() => resolve(null), SEND_PREVIEW_WAIT_MS))]);
  return preview ? url : null;
}

const GIF_KEY = "commy.gif";
const GIF_HELP = {
  klipy: 'To get a key, sign in at <a data-href="https://partner.klipy.com/api-keys">partner.klipy.com</a>, open API Keys, choose Add Platform, then Create API Key and copy it. Test keys are rate limited; use Request Production in the key menu for more.',
  giphy: 'To get a key, sign in at <a data-href="https://developers.giphy.com/dashboard/">developers.giphy.com</a>, choose Create an App, pick API, and copy the key.',
};
const GIF_PROVIDERS = new Set(["klipy", "giphy"]);

function serverHasGifs() {
  return account.server_gifs !== false;
}

function syncGifButton() {
  $("gif-btn").classList.toggle("hidden", !gifSettings().key && !serverHasGifs());
}

function gifSettings() {
  let saved = {};
  try { saved = JSON.parse(localStorage.getItem(GIF_KEY) || "{}"); } catch {}
  if (!GIF_PROVIDERS.has(saved.provider)) saved = { provider: "klipy", key: "" };
  return { provider: "klipy", key: "", ...saved };
}

function klipyUrl(file, sizes) {
  for (const size of sizes) {
    const url = file?.[size]?.gif?.url;
    if (url) return url;
  }
  return null;
}

function klipyRatio(file, sizes) {
  for (const size of sizes) {
    const r = file?.[size]?.gif;
    if (r?.width && r?.height) return Number(r.height) / Number(r.width);
  }
  return 0;
}

async function searchGifs(query, settings = gifSettings()) {
  if (!settings.key) {
    let hits;
    try {
      hits = await invoke("gif_search", { query });
    } catch (e) {
      throw new Error(String(e));
    }
    if (!hits) throw new Error("no-key");
    return hits;
  }
  const q = encodeURIComponent(query.trim());
  if (settings.provider === "klipy") {
    const base = `https://api.klipy.com/api/v1/${encodeURIComponent(settings.key)}/gifs`;
    const common = "per_page=24&page=1&content_filter=medium&format_filter=gif";
    const url = q ? `${base}/search?q=${q}&${common}` : `${base}/trending?${common}`;
    const res = await fetch(url);
    if (res.status === 429) throw new Error("your KLIPY key hit its limit for now (test keys allow 100 searches an hour)");
    const body = await res.json().catch(() => ({}));
    if (!res.ok || body.result === false) {
      const reason = body?.errors?.message?.[0];
      throw new Error(reason || `KLIPY answered ${res.status}`);
    }
    const items = Array.isArray(body?.data) ? body.data : body?.data?.data || [];
    return items.map(item => ({
      preview: klipyUrl(item.file, ["sm", "xs", "md"]),
      ratio: klipyRatio(item.file, ["sm", "xs", "md"]),
      url: klipyUrl(item.file, ["md", "sm", "hd"]),
      title: item.title || "GIF",
    })).filter(g => g.preview && g.url);
  }
  const base = "https://api.giphy.com/v1/gifs";
  const url = q
    ? `${base}/search?api_key=${encodeURIComponent(settings.key)}&q=${q}&limit=24&rating=pg-13`
    : `${base}/trending?api_key=${encodeURIComponent(settings.key)}&limit=24&rating=pg-13`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`GIPHY answered ${res.status}`);
  const data = await res.json();
  return (data.data || []).map(g => ({
    preview: g.images?.fixed_width?.url,
    ratio: Number(g.images?.fixed_width?.height) / Number(g.images?.fixed_width?.width) || 0,
    url: g.images?.downsized_medium?.url || g.images?.original?.url,
    title: g.title || "GIF",
  })).filter(g => g.preview && g.url);
}

const composerGifs = { grid: "gif-grid", attrib: "gif-attrib", search: "gif-search", token: 0, pick: gif => sendGif(gif.url), close: () => closeGifPicker() };
const popGifs = { grid: "emoji-pop-gif-grid", attrib: "emoji-pop-gif-attrib", search: "emoji-pop-gif-search", token: 0, timer: null, pick: null, shown: false, close: () => $("emoji-pop").classList.add("hidden") };

async function renderGifResults(query, view = composerGifs) {
  const grid = $(view.grid);
  const token = ++view.token;
  grid.replaceChildren(el("div", "gif-note", "Searching..."));
  const settings = gifSettings();
  $(view.attrib).textContent = settings.key && settings.provider === "giphy" ? "Powered by GIPHY" : "Powered by KLIPY";
  try {
    const gifs = await searchGifs(query, settings);
    if (token !== view.token) return;
    grid.replaceChildren();
    const cols = [el("div", "gif-col"), el("div", "gif-col")];
    const heights = [0, 0];
    for (const gif of gifs) {
      const ratio = gif.ratio > 0.2 && gif.ratio < 3 ? gif.ratio : 0.75;
      const btn = el("button", "gif-item");
      btn.title = gif.title;
      btn.style.aspectRatio = `1 / ${ratio}`;
      const img = el("img");
      img.src = viaAccountServer(gif.preview);
      img.alt = gif.title;
      img.loading = "lazy";
      img.decoding = "async";
      btn.appendChild(img);
      btn.onclick = () => view.pick && view.pick(gif, btn);
      const target = heights[0] <= heights[1] ? 0 : 1;
      cols[target].appendChild(btn);
      heights[target] += ratio;
    }
    if (gifs.length) grid.append(...cols);
    if (!gifs.length) grid.appendChild(el("div", "gif-note", "No GIFs found."));
  } catch (e) {
    if (token !== view.token) return;
    const note = el("div", "gif-note");
    if (e.message === "no-key") {
      note.textContent = "Your account server does not offer GIF search. Add your own GIF API key to search. ";
      const link = el("button", "btn-link", "Open GIF settings");
      link.onclick = () => {
        view.close();
        openUserSettings("integrations");
      };
      note.appendChild(link);
    } else {
      note.textContent = settings.key
        ? `Could not load GIFs: ${e.message}. Check your API key in Settings, then Integrations. `
        : `Could not load GIFs: ${e.message}. `;
      const retry = el("button", "btn-link", "Try again");
      retry.onclick = () => renderGifResults($(view.search).value, view);
      note.appendChild(retry);
    }
    grid.replaceChildren(note);
  }
}

let gifSearchTimer = null;

function openGifPicker(query = "", target = null) {
  gifTarget = target;
  if (!target && !(state.view === "dm" && state.currentDm) && !(state.view === "server" && state.currentChannel)) return;
  $("emoji-picker").classList.add("hidden");
  $("gif-picker").classList.remove("hidden");
  $("gif-search").value = query;
  $("gif-search").focus();
  renderGifResults(query);
}

function toggleComposerMenu() {
  const menu = $("composer-menu");
  const files = state.view !== "server" || canAttachHere();
  const voice = state.view !== "server" || canSendVoiceHere();
  if (!files && !voice) {
    toast("You can't send files or voice messages in this server");
    return;
  }
  if (!menu.classList.contains("hidden")) {
    menu.classList.add("hidden");
    return;
  }
  closeFloating("composer-menu");
  closeGifPicker();
  $("cm-file").classList.toggle("hidden", !files);
  $("cm-video").classList.toggle("hidden", !files);
  $("cm-gif").classList.toggle("hidden", !files || (!gifSettings().key && !serverHasGifs()));
  $("cm-voice").classList.toggle("hidden", !voice || voiceRec.target !== null);
  $("cm-poll").classList.toggle("hidden", state.view !== "server" || !canSendPolls(state.currentServer));
  placePopup(menu, $("attach-btn"), "above");
}

function openPollModal() {
  if (state.view !== "server" || !state.currentChannel) return;
  $("poll-question").value = "";
  $("poll-rows").replaceChildren();
  addPollRow("");
  addPollRow("");
  $("poll-multi").checked = false;
  const select = $("poll-duration");
  select.replaceChildren(...POLL_DURATIONS.map(d => {
    const option = el("option", null, d.label);
    option.value = d.hours === null ? "" : String(d.hours);
    return option;
  }));
  select.value = String(DEFAULT_POLL_HOURS);
  showModal("modal-poll");
  $("poll-question").focus();
}

function addPollRow(text) {
  const rows = $("poll-rows");
  if (rows.children.length >= MAX_POLL_OPTIONS) return;
  const row = el("div", "poll-row");
  const input = el("input");
  input.type = "text";
  input.maxLength = MAX_POLL_OPTION_CHARS;
  input.autocomplete = "off";
  input.value = text;
  input.onkeydown = e => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    const next = row.nextElementSibling;
    if (next) next.querySelector("input").focus();
    else if (rows.children.length < MAX_POLL_OPTIONS) $("poll-add").onclick();
    else sendPoll();
  };
  const remove = el("button", "poll-row-remove");
  remove.type = "button";
  remove.title = "Remove this answer";
  remove.appendChild(svgIcon("close", 16));
  remove.onclick = () => {
    row.remove();
    syncPollRows();
  };
  row.append(input, remove);
  rows.appendChild(row);
  syncPollRows();
}

function syncPollRows() {
  const rows = [...$("poll-rows").children];
  rows.forEach((row, i) => {
    row.querySelector("input").placeholder = `Answer ${i + 1}`;
    row.querySelector("button").disabled = rows.length <= MIN_POLL_OPTIONS;
  });
  $("poll-add").disabled = rows.length >= MAX_POLL_OPTIONS;
}

async function sendPoll() {
  if (!state.currentServer || !state.currentChannel) return;
  const draft = { question: $("poll-question").value, options: [...$("poll-rows").querySelectorAll("input")].map(i => i.value) };
  const problem = pollDraftProblem(draft);
  if (problem) {
    toast(problem);
    return;
  }
  const hours = Number($("poll-duration").value) || null;
  try {
    await invoke("send_poll", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      question: squashPollText(draft.question),
      options: cleanPollOptions(draft.options),
      multi: $("poll-multi").checked,
      hours,
    });
    hideModals();
    await selectChannel(state.currentChannel);
  } catch (e) {
    toast(String(e));
  }
}

const voiceRec = { target: null, startedAt: 0, timer: null, busy: false };

function voiceRecordingTarget() {
  if (inDmView()) return { dm: state.currentDm.user, name: state.currentDm.name };
  if (state.view === "server" && state.currentServer && state.currentChannel) {
    return { server: state.currentServer.id, channel: state.currentChannel.id };
  }
  return null;
}

function showVoiceRecorder(shown) {
  $("voice-rec").classList.toggle("hidden", !shown);
  $("voice-rec-send").disabled = false;
  $("voice-rec-cancel").disabled = false;
  if (!shown) $("voice-rec-time").textContent = "0:00";
}

function stopVoiceTimer() {
  if (voiceRec.timer) clearInterval(voiceRec.timer);
  voiceRec.timer = null;
}

function tickVoiceRecording() {
  const { label, done } = recordingTimer((Date.now() - voiceRec.startedAt) / 1000);
  $("voice-rec-time").textContent = label;
  if (done) sendVoiceRecording();
}

async function startVoiceRecording() {
  closeFloating();
  const target = voiceRecordingTarget();
  if (!target || voiceRec.target) return;
  if (target.server && !canSendVoiceHere()) {
    toast("You can't send voice messages in this server");
    return;
  }
  try {
    await invoke("start_voice_recording");
  } catch (e) {
    toast(String(e));
    return;
  }
  voiceRec.target = target;
  voiceRec.startedAt = Date.now();
  showVoiceRecorder(true);
  tickVoiceRecording();
  voiceRec.timer = setInterval(tickVoiceRecording, 250);
}

async function cancelVoiceRecording() {
  if (!voiceRec.target || voiceRec.busy) return;
  stopVoiceTimer();
  voiceRec.target = null;
  showVoiceRecorder(false);
  try {
    await invoke("cancel_voice_recording");
  } catch (e) {
    toast(String(e));
  }
}

function renderPoll(message) {
  const poll = message.poll;
  const now = Date.now();
  const closed = pollIsClosed(poll, now);
  const card = el("div", "poll" + (closed ? " closed" : ""));
  card.appendChild(el("div", "poll-question", poll.question));
  const hint = pollHint(poll, now);
  if (hint) card.appendChild(el("div", "poll-hint", hint));
  const list = el("div", "poll-options");
  const shares = pollShares(poll);
  (poll.options || []).forEach((option, i) => {
    const row = el("button", "poll-option" + (option.mine ? " mine" : "") + (poll.multi ? " multi" : ""));
    row.type = "button";
    row.disabled = closed;
    if (!closed) row.title = option.mine ? "Take your vote back" : "Vote for this";
    const fill = el("div", "poll-fill");
    fill.style.width = `${shares[i].percent}%`;
    const mark = el("span", "poll-mark");
    mark.appendChild(svgIcon("check", 14));
    const count = el("span", "poll-count");
    count.appendChild(el("strong", null, `${shares[i].percent}%`));
    count.appendChild(document.createTextNode(` · ${shares[i].votes}`));
    row.append(fill, mark, el("span", "poll-text", option.text), count);
    row.onclick = e => {
      e.stopPropagation();
      votePoll(message, nextPollChoices(poll, i));
    };
    list.appendChild(row);
  });
  card.appendChild(list);
  const foot = el("div", "poll-foot");
  foot.appendChild(el("span", "poll-status", pollFooter(poll, now)));
  const canManage = Boolean(state.currentServer && state.currentServer.can_manage_messages);
  if (state.view === "server" && canClosePoll({ poll, own: isOwnMessage(message), canManage, now })) {
    const close = el("button", "poll-close", "Close poll");
    close.type = "button";
    close.onclick = e => {
      e.stopPropagation();
      closePoll(message);
    };
    foot.appendChild(close);
  }
  card.appendChild(foot);
  return card;
}

async function refreshChannelInPlace() {
  if (!state.currentChannel) return;
  const box = $("messages");
  const keep = box.scrollTop;
  await selectChannel(state.currentChannel);
  box.scrollTop = keep;
}

async function votePoll(message, choices) {
  if (!state.currentServer) return;
  try {
    await invoke("vote_poll", { serverId: state.currentServer.id, messageId: message.id, choices });
    await refreshChannelInPlace();
  } catch (e) {
    toast(String(e));
  }
}

async function closePoll(message) {
  if (!state.currentServer) return;
  if (!confirm("Close this poll now? Nobody can vote after that.")) return;
  try {
    await invoke("close_poll", { serverId: state.currentServer.id, messageId: message.id });
    await refreshChannelInPlace();
  } catch (e) {
    toast(String(e));
  }
}

async function sendVoiceRecording() {
  if (!voiceRec.target || voiceRec.busy) return;
  voiceRec.busy = true;
  stopVoiceTimer();
  $("voice-rec-send").disabled = true;
  $("voice-rec-cancel").disabled = true;
  const target = voiceRec.target;
  try {
    if (target.dm) {
      const sent = await invoke("send_dm_voice_message", { userId: target.dm });
      if (sent && inDmView() && state.currentDm.user === target.dm) await selectDm(target.dm, target.name);
    } else {
      const sent = await invoke("send_voice_message", { serverId: target.server, channelId: target.channel });
      if (sent && state.currentChannel && state.currentChannel.id === target.channel) await selectChannel(state.currentChannel);
    }
  } catch (e) {
    toast(String(e));
  }
  voiceRec.busy = false;
  voiceRec.target = null;
  showVoiceRecorder(false);
}

async function attachFromMenu(kind) {
  closeFloating();
  if (inDmView()) {
    try {
      const sent = await invoke("attach_dm_file", { userId: state.currentDm.user, kind });
      if (sent) await selectDm(state.currentDm.user, state.currentDm.name);
    } catch (e) {
      toast(String(e));
    }
    return;
  }
  if (!state.currentServer || !state.currentChannel) return;
  try {
    const sent = await invoke("attach_file", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      kind,
    });
    if (sent) await selectChannel(state.currentChannel);
  } catch (e) {
    toast(String(e));
  }
}

function closeGifPicker() {
  $("gif-picker").classList.add("hidden");
}

async function sendGif(url) {
  closeGifPicker();
  const dmTarget = gifTarget || (state.view === "dm" && state.currentDm ? { user: state.currentDm.user, name: state.currentDm.name } : null);
  gifTarget = null;
  if (dmTarget) {
    try {
      await invoke("open_dm", { userId: dmTarget.user });
      const sent = await invoke("send_dm", { userId: dmTarget.user, content: url });
      if (state.view === "dm" && state.currentDm && state.currentDm.user === dmTarget.user) {
        await selectDm(dmTarget.user, dmTarget.name);
      } else {
        dmSentToast(sent, dmTarget.name, `GIF sent to ${dmTarget.name}`);
      }
    } catch (e) {
      toast(String(e));
    }
    return;
  }
  if (!state.currentServer || !state.currentChannel) return;
  const replyTo = state.replyingTo ? state.replyingTo.id : null;
  $("messages").dataset.view = "";
  cancelReply();
  try {
    await invoke("send_message", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      content: url,
      replyTo,
    });
    await selectChannel(state.currentChannel);
  } catch (e) {
    toast(String(e));
  }
}

async function askForMicrophoneOnPhones() {
  if (!document.body.classList.contains("mobile-app") || !navigator.mediaDevices) return;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true, video: false });
    stream.getTracks().forEach(track => track.stop());
  } catch {
    toast("Commy needs the microphone for voice. You can allow it in your phone's settings.");
  }
}

async function toggleVoice(channel) {
  if (!state.currentServer) return;
  const leaving = state.voiceChannelId === channel.id;
  if (!leaving) await askForMicrophoneOnPhones();
  try {
    await invoke("set_voice_channel", {
      serverId: state.currentServer.id,
      channelId: leaving ? null : channel.id,
    });
    state.voiceChannelId = leaving ? null : channel.id;
    clearSpeaking();
    if (!state.voiceChannelId) {
      stopCameraTracks();
      state.cameraOn = false;
      cam.watching.clear();
    }
    state.voiceServer = leaving
      ? null
      : { id: state.currentServer.id, name: state.currentServer.name, channelName: channel.name, activities_enabled: Boolean(state.currentServer.activities_enabled) };
    if (leaving) {
      state.sharing = false;
      state.shareSource = null;
    }
    toast(leaving ? "Disconnected from voice" : isStageChannel(channel) ? `Joined the stage: ${channel.name}` : `Joined voice: ${channel.name}`);
    playSound(leaving ? "leave" : "join");
    renderShareButton();
    await selectServer(state.currentServer.id);
  } catch (e) {
    toast(String(e));
  }
}

async function toggleShare() {
  if (!state.voiceServer || !state.voiceChannelId) return;
  if (state.sharing) {
    try {
      await invoke("stop_screen_share", { serverId: state.voiceServer.id });
      state.sharing = false;
      state.shareSource = null;
      if (state.stageMode === "self") closeStage();
      toast("Stopped sharing");
      renderShareButton();
    } catch (e) {
      toast(String(e));
    }
    return;
  }
  await openSharePicker();
}

const SHARE_QUALITY_KEY = "commy.shareQuality";
const SHARE_QUALITY_HINTS = {
  smooth: "Best for games and video. Lower resolution, more frames.",
  balanced: "A good default for most screens.",
  sharp: "Best for code and documents. Crisp text, fewer frames.",
};

const sharePicker = { sources: [], kind: "monitor", picked: null, thumbs: new Map(), token: 0, audioSupported: null };

function shareAudioWanted(supported = true) {
  let saved = null;
  try { saved = localStorage.getItem(SHARE_AUDIO_KEY); } catch {}
  return shareAudioChoice(saved, supported);
}

async function renderShareAudioRow() {
  if (sharePicker.audioSupported === null) {
    try { sharePicker.audioSupported = Boolean(await invoke("share_audio_supported")); } catch { sharePicker.audioSupported = false; }
  }
  $("share-audio-row").classList.toggle("hidden", !sharePicker.audioSupported);
  $("share-audio").checked = shareAudioWanted(sharePicker.audioSupported);
}

function shareQuality() {
  try {
    const saved = localStorage.getItem(SHARE_QUALITY_KEY);
    if (saved in SHARE_QUALITY_HINTS) return saved;
  } catch {}
  return "balanced";
}

function setShareQuality(quality) {
  try { localStorage.setItem(SHARE_QUALITY_KEY, quality); } catch {}
  for (const btn of $("share-quality").querySelectorAll("button")) {
    btn.classList.toggle("active", btn.dataset.q === quality);
  }
  $("share-quality-hint").textContent = SHARE_QUALITY_HINTS[quality];
}

async function openSharePicker() {
  if (!state.voiceServer) return;
  const token = ++sharePicker.token;
  sharePicker.sources = [];
  sharePicker.picked = null;
  sharePicker.thumbs.clear();
  sharePicker.kind = "monitor";
  setShareQuality(shareQuality());
  renderShareAudioRow();
  $("share-go").textContent = state.sharing ? "Switch Stream" : "Go Live";
  const list = $("share-source-list");
  list.innerHTML = "";
  list.appendChild(el("div", "share-loading", "Looking for screens and windows..."));
  renderShareTabs();
  showModal("modal-share");
  try {
    sharePicker.sources = await invoke("list_share_sources");
    if (token !== sharePicker.token) return;
    const game = sharePicker.sources.find(s => s.game);
    const primary = game || sharePicker.sources.find(s => s.is_primary) || sharePicker.sources[0];
    if (primary) sharePicker.picked = primary;
    if (game) {
      sharePicker.kind = "window";
      setShareQuality("smooth");
      $("share-go").textContent = state.sharing ? "Switch Stream" : `Share ${game.game}`;
    }
    renderShareGrid();
    loadShareThumbs(token);
  } catch (e) {
    toast(String(e));
    hideModals();
  }
}

function renderShareTabs() {
  for (const tab of $("share-tabs").querySelectorAll(".share-tab")) {
    const count = sharePicker.sources.filter(s => s.kind === tab.dataset.kind).length;
    tab.classList.toggle("active", tab.dataset.kind === sharePicker.kind);
    tab.textContent = `${tab.dataset.kind === "monitor" ? "Screens" : "Applications"}${count ? ` (${count})` : ""}`;
  }
}

function renderShareGrid() {
  renderShareTabs();
  const list = $("share-source-list");
  list.innerHTML = "";
  const sources = sharePicker.sources.filter(s => s.kind === sharePicker.kind);
  for (const source of sources) {
    const card = el("button", "share-card");
    const key = `${source.kind}:${source.id}`;
    card.dataset.key = key;
    if (sharePicker.picked && `${sharePicker.picked.kind}:${sharePicker.picked.id}` === key) {
      card.classList.add("picked");
    }
    const thumb = el("div", "share-thumb");
    const src = sharePicker.thumbs.get(key);
    if (src) {
      const img = el("img");
      img.src = src;
      thumb.appendChild(img);
    } else {
      thumb.appendChild(svgIcon("screen", 32));
    }
    card.appendChild(thumb);
    const label = el("div", "share-card-label");
    label.appendChild(el("span", "share-card-name", source.name));
    if (source.is_primary) label.appendChild(el("span", "kind", "primary"));
    if (source.game) label.appendChild(el("span", "kind playing", "playing"));
    card.appendChild(label);
    card.title = source.name;
    card.onclick = () => {
      sharePicker.picked = source;
      for (const other of list.querySelectorAll(".share-card")) other.classList.toggle("picked", other === card);
      $("share-go").disabled = false;
    };
    card.ondblclick = () => goLive();
    list.appendChild(card);
  }
  if (!sources.length) {
    list.appendChild(el("div", "share-loading", sharePicker.kind === "monitor"
      ? "No screens found."
      : "No open windows found. Minimized windows are hidden."));
  }
  const pickedVisible = sharePicker.picked && sharePicker.picked.kind === sharePicker.kind;
  $("share-go").disabled = !pickedVisible;
}

async function loadShareThumbs(token) {
  const ordered = [...sharePicker.sources].sort((a, b) =>
    (a.kind === sharePicker.kind ? 0 : 1) - (b.kind === sharePicker.kind ? 0 : 1));
  for (const source of ordered) {
    if (token !== sharePicker.token || $("modal-share").classList.contains("hidden")) return;
    try {
      const jpeg = await invoke("share_source_preview", { sourceKind: source.kind, sourceId: source.id });
      const key = `${source.kind}:${source.id}`;
      sharePicker.thumbs.set(key, `data:image/jpeg;base64,${jpeg}`);
      const card = $("share-source-list").querySelector(`[data-key="${key}"] .share-thumb`);
      if (card) {
        const img = el("img");
        img.src = sharePicker.thumbs.get(key);
        card.replaceChildren(img);
      }
    } catch {}
  }
}

function goLive() {
  const source = sharePicker.picked;
  if (!source || source.kind !== sharePicker.kind) return;
  startShareWith(source, shareQuality(), shareAudioWanted(sharePicker.audioSupported === true));
}

async function startShareWith(source, quality, audio = shareAudioWanted()) {
  hideModals();
  sharePicker.token++;
  try {
    if (state.sharing) await invoke("stop_screen_share", { serverId: state.voiceServer.id });
    await invoke("start_screen_share", {
      serverId: state.voiceServer.id,
      sourceKind: source.kind,
      sourceId: source.id,
      quality,
      audio,
    });
    state.sharing = true;
    state.shareSource = source.name;
    state.shareQuality = quality;
    toast(liveToastText(source.name, audio));
    renderShareButton();
    startOwnPreview();
  } catch (e) {
    toast(String(e));
  }
}

let ownPreviewTimer = null;
let ownFpsSample = null;
let ownFrameAt = 0;

function showOwnFrame(frame) {
  if (!state.sharing) return;
  ownFrameAt = performance.now();
  const src = `data:image/jpeg;base64,${frame.jpeg}`;
  $("vp-preview-img").src = src;
  $("vp-preview-empty").classList.add("hidden");
  if (state.stageMode === "self") showStageFrame(src, frame.width, frame.height);
}

function startOwnPreview() {
  if (ownPreviewTimer) return;
  ownFpsSample = null;
  ownFrameAt = 0;
  $("vp-preview-img").removeAttribute("src");
  $("vp-preview-empty").classList.remove("hidden");
  const tick = async () => {
    if (!state.sharing) {
      clearInterval(ownPreviewTimer);
      ownPreviewTimer = null;
      return;
    }
    try {
      const own = await invoke("own_share_frame");
      if (own.jpeg && performance.now() - ownFrameAt > 2000) {
        showOwnFrame(own);
      }
      $("vp-viewers").textContent = own.viewers === 1 ? "1 watching" : `${own.viewers} watching`;
      $("vp-viewers").classList.toggle("hidden", !own.viewers);
      if (state.stageMode === "self") {
        $("stage-viewers").textContent = own.viewers === 1 ? "1 watching" : `${own.viewers} watching`;
        $("stage-viewers").classList.toggle("hidden", !own.viewers);
      }
      const now = performance.now();
      if (ownFpsSample && own.frames >= ownFpsSample.frames) {
        const fps = Math.round((own.frames - ownFpsSample.frames) / ((now - ownFpsSample.at) / 1000));
        $("vp-live-source").textContent = liveSourceText(state.shareSource, own.audio, fps);
      }
      ownFpsSample = { frames: own.frames, at: now };
    } catch {}
  };
  tick();
  ownPreviewTimer = setInterval(tick, 1000);
}

const stageStats = { times: [], width: 0, height: 0 };

const STREAM_STALL_MS = 6000;
setInterval(() => {
  if (!state.watchingUser || !stageStats.lastAt) return;
  if (performance.now() - stageStats.lastAt < STREAM_STALL_MS) return;
  stageStats.lastAt = 0;
  toast(state.watchingName ? `${state.watchingName}'s stream ended` : "The stream ended");
  stopWatching();
}, 2000);

function openStage(mode, label) {
  state.stageMode = mode;
  stageStats.times = [];
  $("video-label").textContent = label;
  $("stage-stats").textContent = "";
  $("stage-viewers").textContent = "";
  $("stage-viewers").classList.add("hidden");
  $("video-frame").removeAttribute("src");
  $("stage-wait").classList.remove("hidden");
  $("video-pane").classList.remove("hidden");
  $("video-pane").classList.toggle("self", mode === "self");
  $("video-close").title = mode === "self" ? "Close Preview" : "Stop Watching";
  renderStageVolume(streamVolumePercent(state.voice));
}

function renderStageVolume(percent) {
  const range = $("stage-volume-range");
  range.value = String(percent);
  syncRangeFill(range);
  $("stage-volume").title = streamVolumeTitle(percent);
}

async function setStreamVolume(percent) {
  try {
    state.voice = await invoke("set_stream_volume", { volume: streamVolumeGain(percent) });
  } catch (e) {
    toast(String(e));
  }
}

function closeStage() {
  state.stageMode = null;
  if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
  document.querySelector(".main").classList.remove("stage-focus");
  $("video-pane").classList.add("hidden");
  $("video-frame").removeAttribute("src");
}

function showStageFrame(src, width, height) {
  stageStats.lastAt = performance.now();
  $("video-frame").src = src;
  $("stage-wait").classList.add("hidden");
  const now = performance.now();
  stageStats.times.push(now);
  while (stageStats.times.length && now - stageStats.times[0] > 3000) stageStats.times.shift();
  const span = (now - stageStats.times[0]) / 1000;
  const fps = span > 0 ? Math.round((stageStats.times.length - 1) / span) : 0;
  const res = width && height ? `${width}x${height}` : "";
  $("stage-stats").textContent = [res, fps ? `${fps} fps` : ""].filter(Boolean).join(" \u00b7 ");
}

function toggleStageFocus() {
  const focused = document.querySelector(".main").classList.toggle("stage-focus");
  $("stage-focus").title = focused ? "Exit Focus Mode" : "Focus Mode";
  setIcon($("stage-focus"), focused ? "collapse" : "expand", 20);
}

function toggleStageFullscreen() {
  if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
  else $("stage-view").requestFullscreen().catch(e => toast(String(e)));
}

async function uploadFileObject(file) {
  const dm = inDmView();
  if (!dm && (state.view !== "server" || !state.currentServer || !state.currentChannel)) {
    toast("Open a channel or conversation to share files");
    return;
  }
  if (!dm && !canAttachHere()) {
    toast("You can't send files or images in this server");
    return;
  }
  if (file.size > 512 * 1024 * 1024) {
    toast("File is too large (512 MB max)");
    return;
  }
  const buffer = await file.arrayBuffer();
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
  }
  const name = file.name && file.name !== "image.png"
    ? file.name
    : `pasted-${Date.now()}.png`;
  try {
    if (dm) {
      await invoke("attach_dm_bytes", { userId: state.currentDm.user, filename: name, data: btoa(binary) });
      await selectDm(state.currentDm.user, state.currentDm.name);
      return;
    }
    await invoke("attach_bytes", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      filename: name,
      data: btoa(binary),
    });
    await selectChannel(state.currentChannel);
  } catch (e) {
    toast(String(e));
  }
}

async function watchScreen(userId, name) {
  const serverId = state.currentServer ? state.currentServer.id : state.voiceServer && state.voiceServer.id;
  if (!serverId) return;
  try {
    if (state.watchingUser && state.watchingUser !== userId) await stopWatching();
    openStage("watch", `${name}'s screen`);
    await invoke("watch_screen", { serverId, userId });
    state.watchingUser = userId;
    state.watchingName = name;
    renderVoicePanel();
  } catch (e) {
    closeStage();
    toast(String(e));
  }
}

async function stopWatching() {
  if (state.watchingUser) {
    try {
      await invoke("stop_watching", { userId: state.watchingUser });
    } catch (e) {}
  }
  state.watchingUser = null;
  state.watchingName = null;
  if (state.stageMode === "watch") closeStage();
  renderVoicePanel();
}

async function closeStageButton() {
  if (state.stageMode === "watch") await stopWatching();
  else closeStage();
}

async function openOwnPreview() {
  if (!state.sharing) return;
  if (state.watchingUser) await stopWatching();
  openStage("self", "Your stream preview");
}

function wireEvents() {
  $("signin-go").onclick = beginSignIn;
  $("signin-reopen").onclick = () => {
    if (signInUrl) invoke("open_url", { url: signInUrl }).catch(e => toast(String(e)));
  };
  $("signin-cancel").onclick = async () => {
    await invoke("account_cancel_sign_in");
    showSignIn();
  };
  $("signin-server-select").addEventListener("change", e => {
    const value = e.target.value;
    if (value === "custom") {
      openServerEdit();
      return;
    }
    useAccountServer(value === "default" ? null : value);
  });
  $("signin-server-input").addEventListener("input", syncServerNote);
  $("signin-server-input").addEventListener("keydown", e => {
    if (e.key === "Enter") $("signin-server-save").click();
    if (e.key === "Escape") {
      e.stopPropagation();
      closeServerEdit();
    }
  });
  $("signin-server-save").onclick = () => {
    const typed = $("signin-server-input").value.trim();
    if (typed) useAccountServer(typed);
    else showServerProblem("signin.server.bad.domain");
  };
  $("signin-server-cancel").onclick = closeServerEdit;
  $("claim-keep").onclick = () => answerClaim(true);
  $("claim-fresh").onclick = () => answerClaim(false);
  $("claim-retry").onclick = () => openAccountFolder();
  $("acct-sign-out").onclick = signOutOfCurrent;
  $("pp-switch-account").onclick = openAccounts;
  $("account-add").onclick = () => {
    addingAccount = true;
    showSignIn();
  };
  $("account-add-icon").replaceChildren(svgIcon("plus", 18));
  $("signin-back").onclick = async () => {
    addingAccount = false;
    await invoke("account_cancel_sign_in").catch(() => {});
    await openAccounts();
  };
  $("username-save").onclick = async () => {
    const username = $("username-input").value.trim();
    if (!username) return;
    try {
      state.user = await invoke("init_user", { username });
      hideModals();
      renderUserPanel();
      await refreshServers();
      await loadReadState();
      await loadPins();
      await loadSaved();
      if (railServers().length) await selectServer(railServers()[0].id);
      else await enterDmMode();
      if (!(await takePendingInviteLink())) await offerTrayPrompt();
    } catch (e) {
      toast(String(e));
    }
  };
  $("username-input").addEventListener("keydown", e => {
    if (e.key === "Enter") $("username-save").click();
  });

  $("add-server-btn").onclick = () => showModal("modal-add-server");
  wireDiscover();
  wireVoiceInvites();
  $("create-server-open").onclick = () => openCreateServer();
  $("create-server-back").onclick = () => showModal("modal-add-server");
  $("template-code-input").addEventListener("input", readTemplateCode);
  $("template-code-change").onclick = () => {
    templatePicker.preview = null;
    templatePicker.code = null;
    $("template-code-input").value = "";
    renderTemplatePreview();
    $("template-code-input").focus();
  };
  $("create-name-input").addEventListener("keydown", e => {
    if (e.key === "Enter") $("create-server-save").click();
  });

  $("create-server-save").onclick = async () => {
    const name = $("create-name-input").value.trim();
    const template = chosenTemplate();
    if (!name) return $("create-name-input").focus();
    if (!template) return $("template-code-input").focus();
    const save = $("create-server-save");
    if (save.disabled) return;
    save.disabled = true;
    try {
      const server = await invoke("create_server", { name, template });
      hideModals();
      $("create-name-input").value = "";
      await refreshServers();
      await selectServer(server.id);
    } catch (e) {
      toast(String(e));
    } finally {
      save.disabled = false;
    }
  };

  $("server-template-copy").onclick = async () => {
    if (!state.currentServer) return;
    try {
      const code = await invoke("export_server_template", { serverId: state.currentServer.id });
      await navigator.clipboard.writeText(code);
      toast("Template code copied. Anyone can paste it under Create a Server.");
    } catch (e) {
      toast(String(e));
    }
  };

  $("join-server-save").onclick = async () => {
    const ticket = $("join-ticket-input").value.trim();
    if (!ticket) return;
    if (looksLikeTemplateCode(ticket)) {
      $("join-ticket-input").value = "";
      return openCreateServer(ticket);
    }
    if (looksLikeInviteLink(ticket)) {
      $("join-ticket-input").value = "";
      return openInviteLink(ticket);
    }
    try {
      const summary = await invoke("join_server", { ticket });
      hideModals();
      $("join-ticket-input").value = "";
      await enterJoinedServer(summary);
    } catch (e) {
      toast(String(e));
    }
  };

  $("invite-btn").onclick = async () => {
    if (!state.currentServer || !state.currentServer.can_invite) return;
    try {
      const invite = await invoke("get_invite", { serverId: state.currentServer.id });
      $("invite-text").value = invite;
      resetInviteDialog();
      $("invite-friend-search").value = "";
      state.invitedNow = new Set();
      renderInvitePeople();
      showModal("modal-invite");
      await refreshPeople();
      renderInvitePeople();
    } catch (e) {
      toast(String(e));
    }
  };

  $("invite-friend-search").addEventListener("input", renderInvitePeople);

  $("invite-copy").onclick = async () => {
    await navigator.clipboard.writeText($("invite-text").value);
    toast("Invite copied to clipboard");
  };

  $("settings-btn").onclick = () => {
    openUserSettings("voice");
    showSettingsMenuOnPhone("user-settings");
  };
  $("settings-close").onclick = closeSettingsLayers;
  $("server-settings-close").onclick = closeSettingsLayers;
  for (const layer of document.querySelectorAll(".settings-layer")) {
    for (const btn of layer.querySelectorAll(".settings-tab[data-tab]")) {
      btn.onclick = () => selectSettingsTab(layer.id, btn.dataset.tab);
    }
  }

  const appearance = loadAppearance();
  applyAppearance(appearance);
  $("chat-size").value = appearance.chatSize;
  $("chat-size-label").textContent = `${appearance.chatSize}px`;
  $("compact-toggle").checked = appearance.compact;
  $("chat-size").addEventListener("input", e => {
    appearance.chatSize = Number(e.target.value);
    $("chat-size-label").textContent = `${appearance.chatSize}px`;
    saveAppearance(appearance);
  });
  $("compact-toggle").addEventListener("change", e => {
    appearance.compact = e.target.checked;
    saveAppearance(appearance);
  });

  $("mic-test-btn").onclick = async () => {
    try {
      if (state.micTesting) {
        await stopMicTest();
      } else {
        await invoke("start_mic_test");
        state.micTesting = true;
        $("mic-test-btn").textContent = "Stop Test";
        toast("Mic test running: you should hear yourself");
      }
    } catch (e) {
      toast(String(e));
    }
  };

  $("emoji-btn").onclick = () => {
    const picker = $("emoji-picker");
    const opening = picker.classList.contains("hidden");
    picker.classList.toggle("hidden");
    if (opening) setPickerKind(pickerKind);
  };
  for (const tab of $("picker-tabs").querySelectorAll("button")) tab.onclick = () => setPickerKind(tab.dataset.kind);
  $("emoji-search").addEventListener("input", e => renderEmojiGrid(e.target.value));
  document.addEventListener("mousedown", e => {
    const picker = $("emoji-picker");
    if (!picker.classList.contains("hidden") &&
        !picker.contains(e.target) && e.target !== $("emoji-btn") && !$("emoji-btn").contains(e.target)) {
      picker.classList.add("hidden");
    }
  });

  $("server-settings-btn").onclick = async () => {
    await openServerSettings();
    showSettingsMenuOnPhone("server-settings");
  };
  $("emoji-upload").onclick = uploadEmoji;
  $("emoji-browse").onclick = () => openCatalog("emojis");
  $("sticker-browse").onclick = () => openCatalog("stickers");
  $("sticker-upload").onclick = () => openCatalog("stickers", { upload: true });
  $("emoticon-add").onclick = () => openEmoticonModal();
  $("emoticon-save").onclick = saveEmoticon;
  $("emoticon-cancel").onclick = hideModals;
  for (const id of ["emoticon-text", "emoticon-name"]) {
    $(id).addEventListener("keydown", e => {
      if (e.key === "Enter") saveEmoticon();
    });
  }
  $("emoticon-text").addEventListener("input", () => {
    const named = builtinEmoticons().find(f => f.text === $("emoticon-text").value.trim());
    if (named && !$("emoticon-name").value) $("emoticon-name").value = named.name;
  });
  $("catalog-close").replaceChildren(svgIcon("close", 18));
  $("catalog-close").onclick = hideModals;
  $("catalog-add").onclick = addFromCatalog;
  $("catalog-upload").onclick = uploadToCatalog;
  $("catalog-name").addEventListener("keydown", e => {
    if (e.key === "Enter") addFromCatalog();
  });
  $("catalog-search").addEventListener("input", e => {
    clearTimeout(catalog.timer);
    catalog.timer = setTimeout(() => {
      catalog.query = e.target.value.trim();
      catalog.page = 0;
      loadCatalog();
    }, 300);
  });
  const saveExpressionSharing = async () => {
    const server = state.currentServer;
    if (!server) return;
    try {
      await invoke("set_expression_sharing", {
        serverId: server.id,
        stickers: $("sticker-share-toggle").checked,
        emoticons: $("emoticon-share-toggle").checked,
      });
      await refreshExpressions();
    } catch (e) {
      toast(String(e));
    }
  };
  $("sticker-share-toggle").addEventListener("change", saveExpressionSharing);
  $("emoticon-share-toggle").addEventListener("change", saveExpressionSharing);
  $("sound-upload").onclick = () => openSoundUpload(state.currentServer);
  wireSoundUpload();
  const saveExpressionSettings = async () => {
    const server = state.currentServer;
    if (!server) return;
    try {
      await invoke("set_expression_settings", {
        serverId: server.id,
        emojisShareable: $("emoji-share-toggle").checked,
        soundboardOpen: $("sound-open-toggle").checked,
      });
      await refreshExpressions();
    } catch (e) {
      toast(String(e));
      openServerSettings();
    }
  };
  $("emoji-share-toggle").addEventListener("change", saveExpressionSettings);
  $("sound-share-toggle").addEventListener("change", async e => {
    const server = state.currentServer;
    if (!server) return;
    try {
      await invoke("set_sound_sharing", { serverId: server.id, shareable: e.target.checked });
      await refreshExpressions();
    } catch (err) {
      toast(String(err));
      openServerSettings();
    }
  });
  $("server-activities-toggle").addEventListener("change", async e => {
    const server = state.currentServer;
    if (!server) return;
    try {
      await invoke("set_activities_enabled", { serverId: server.id, enabled: e.target.checked });
      server.activities_enabled = e.target.checked;
      renderVoicePanelActivity();
    } catch (err) {
      toast(String(err));
      openServerSettings();
    }
  });
  $("server-public-toggle").addEventListener("change", async e => {
    const server = state.currentServer;
    if (!server) return;
    try {
      await invoke("set_server_public", { serverId: server.id, public: e.target.checked });
      server.public = e.target.checked;
      paintDiscoverySettings(server);
    } catch (err) {
      toast(String(err));
      e.target.checked = Boolean(server.public);
    }
  });
  $("sound-open-toggle").addEventListener("change", saveExpressionSettings);

  $("vp-soundboard").replaceChildren(svgIcon("soundboard", 16), el("span", null, "Soundboard"));
  $("vp-soundboard").onclick = toggleSoundboardPop;
  $("soundboard-toggle").addEventListener("change", e => {
    saveSoundboardPrefs({ ...soundboardPrefs(), enabled: e.target.checked });
  });
  $("soundboard-volume").addEventListener("input", e => {
    saveSoundboardPrefs({ ...soundboardPrefs(), volume: Number(e.target.value) / 100 });
    $("soundboard-volume-label").textContent = `${e.target.value}%`;
    syncRangeFill(e.target);
  });
  document.addEventListener("mousedown", e => {
    const pop = $("soundboard-pop");
    if (!pop.classList.contains("hidden") && !pop.contains(e.target) && !$("vp-soundboard").contains(e.target)) {
      pop.classList.add("hidden");
    }
  });
  $("server-afk-timeout").addEventListener("change", async e => {
    if (!state.currentServer) return;
    const minutes = Number(e.target.value) || null;
    try {
      await invoke("set_server_afk_timeout", { serverId: state.currentServer.id, minutes });
      toast(minutes ? `Idle members are moved after ${minutes} minute${minutes === 1 ? "" : "s"}` : "Server AFK limit turned off");
      await refreshCurrentServer();
    } catch (err) {
      toast(String(err));
    }
  });
  $("afk-channel-select").addEventListener("change", async e => {
    if (!state.currentServer) return;
    const channelId = e.target.value || null;
    try {
      await invoke("set_afk_channel", { serverId: state.currentServer.id, channelId });
      const name = channelId ? e.target.selectedOptions[0].textContent : null;
      toast(name ? `AFK channel set to ${name}` : "AFK channel turned off");
      await refreshCurrentServer();
      openServerSettings();
    } catch (err) {
      toast(String(err));
    }
  });
  $("rename-save").onclick = async () => {
    if (!state.currentServer) return;
    try {
      await invoke("rename_server", {
        serverId: state.currentServer.id,
        name: $("rename-input").value,
      });
      toast("Server renamed");
      await refreshServers();
      await refreshCurrentServer();
      openServerSettings();
    } catch (e) {
      toast(String(e));
    }
  };
  $("new-channel-save").onclick = async () => {
    if (!state.currentServer) return;
    const name = $("new-channel-input").value.trim();
    if (!name) return;
    try {
      await invoke("create_channel", {
        serverId: state.currentServer.id,
        name,
        kind: $("new-channel-kind").value,
      });
      $("new-channel-input").value = "";
      await refreshCurrentServer();
      openServerSettings();
    } catch (e) {
      toast(String(e));
    }
  };
  $("new-role-open").onclick = () => {
    const card = $("new-role-card");
    if (!card.classList.contains("hidden") || !state.currentServer) {
      card.classList.add("hidden");
      return;
    }
    card.replaceChildren(roleForm(state.currentServer, null, () => card.classList.add("hidden")));
    card.classList.remove("hidden");
    card.querySelector("input[type=text]").focus();
  };

  $("leave-server-btn").onclick = () => state.currentServer && openLeaveServer(state.currentServer.id);
  $("si-close").onclick = hideModals;
  $("si-leave").onclick = () => state.currentServer && openLeaveServer(state.currentServer.id);

  $("user-chip").onclick = e => {
    e.stopPropagation();
    toggleProfilePopover();
  };
  $("account-display-name").addEventListener("input", e => {
    $("account-preview-name").textContent = e.target.value || (state.user ? state.user.username : "");
  });
  $("account-save").onclick = async () => {
    try {
      state.user = await invoke("update_profile", {
        displayName: $("account-display-name").value || null,
        customStatus: encodeCustomEmojis($("account-status").value) || null,
        presence: $("account-presence").value,
        statusEmoji: accountStatusEmoji,
      });
      renderUserPanel();
      if (state.currentServer) await refreshCurrentServer();
      toast("Profile updated");
    } catch (e) {
      toast(String(e));
    }
  };

  $("nc-toggle").onchange = async e => {
    state.voice = await invoke("set_noise_cancellation", { enabled: e.target.checked });
    toast(`Noise cancellation ${state.voice.noise_cancellation ? "on" : "off"}`);
    renderVoicePanel();
  };

  $("ec-toggle").onchange = async e => {
    state.voice = await invoke("set_echo_cancellation", { enabled: e.target.checked });
    toast(`Echo cancellation ${state.voice.echo_cancellation ? "on" : "off"}`);
  };

  $("agc-toggle").onchange = async e => {
    state.voice = await invoke("set_auto_level", { enabled: e.target.checked });
    toast(`Automatic mic level ${state.voice.auto_level ? "on" : "off"}`);
  };

  $("mute-btn").onclick = async () => {
    state.voice = await invoke("set_muted", { muted: !state.voice.muted });
    renderUserPanel();
    animateToggle($("mute-btn"), state.voice.muted);
  };

  $("deafen-btn").onclick = async () => {
    state.voice = await invoke("set_deafened", { deafened: !state.voice.deafened });
    renderUserPanel();
    animateToggle($("deafen-btn"), state.voice.deafened);
  };
  $("mute-caret").replaceChildren(svgIcon("chevronDown", 12));
  $("deafen-caret").replaceChildren(svgIcon("chevronDown", 12));
  $("mute-caret").onclick = e => toggleAudioPop("input", e.currentTarget);
  $("deafen-caret").onclick = e => toggleAudioPop("output", e.currentTarget);
  $("ap-device").addEventListener("change", async e => {
    const which = $("audio-pop").dataset.which;
    try {
      state.voice = await invoke("set_audio_devices", {
        input: which === "input" ? e.target.value || null : state.voice.input_device || null,
        output: which === "output" ? e.target.value || null : state.voice.output_device || null,
      });
      toast(which === "input" ? "Microphone changed" : "Speakers changed");
    } catch (err) {
      toast(String(err));
    }
  });
  $("ap-volume").addEventListener("input", e => {
    $("ap-volume-label").textContent = `${e.target.value}%`;
  });
  $("ap-volume").addEventListener("change", async e => {
    const which = $("audio-pop").dataset.which;
    const value = Number(e.target.value) / 100;
    try {
      state.voice = await invoke("set_volumes", {
        outputVolume: which === "output" ? value : state.voice.output_volume ?? 1,
        inputGain: which === "input" ? value : state.voice.input_gain ?? 1,
      });
    } catch (err) {
      toast(String(err));
    }
  });
  $("ap-settings").onclick = () => {
    closeFloating();
    openUserSettings("voice");
  };

  $("attach-btn").onclick = toggleComposerMenu;
  $("cm-file").prepend(svgIcon("uploadFile", 18));
  $("cm-video").prepend(svgIcon("movie", 18));
  $("cm-gif").prepend(svgIcon("gifBox", 18));
  $("cm-poll").prepend(svgIcon("poll", 18));
  $("cm-voice").prepend(svgIcon("mic", 18));
  $("cm-file").onclick = () => attachFromMenu("file");
  $("cm-video").onclick = () => attachFromMenu("video");
  $("cm-voice").onclick = startVoiceRecording;
  $("voice-rec-cancel").onclick = cancelVoiceRecording;
  $("voice-rec-send").onclick = sendVoiceRecording;
  $("cm-gif").onclick = () => {
    closeFloating();
    openGifPicker("");
  };
  $("cm-poll").onclick = () => {
    closeFloating();
    openPollModal();
  };
  $("poll-cancel").onclick = hideModals;
  $("poll-send").onclick = sendPoll;
  $("poll-add").onclick = () => {
    addPollRow("");
    const last = $("poll-rows").lastElementChild;
    if (last) last.querySelector("input").focus();
  };
  $("share-btn").onclick = toggleShare;
  $("video-close").onclick = closeStageButton;
  setIcon($("video-close"), "close", 20);
  setIcon($("stage-focus"), "expand", 20);
  setIcon($("stage-fullscreen"), "fullscreen", 20);
  setIcon($("stage-volume-icon"), "speaker", 18);
  $("stage-volume-range").oninput = () => renderStageVolume(Number($("stage-volume-range").value));
  $("stage-volume-range").onchange = () => setStreamVolume(Number($("stage-volume-range").value));
  $("share-audio").onchange = () => {
    try { localStorage.setItem(SHARE_AUDIO_KEY, shareAudioSaveValue($("share-audio").checked)); } catch {}
  };
  $("stage-focus").onclick = toggleStageFocus;
  $("stage-fullscreen").onclick = toggleStageFullscreen;
  $("stage-view").ondblclick = toggleStageFullscreen;
  $("vp-preview").onclick = openOwnPreview;

  for (const tab of $("share-tabs").querySelectorAll(".share-tab")) {
    tab.onclick = () => {
      sharePicker.kind = tab.dataset.kind;
      renderShareGrid();
    };
  }
  for (const btn of $("share-quality").querySelectorAll("button")) {
    btn.onclick = () => setShareQuality(btn.dataset.q);
  }
  $("share-go").onclick = goLive;
  initActivityUi();
  $("vp-game-share").onclick = async () => {
    try {
      const sources = await invoke("list_share_sources");
      const game = sources.find(s => s.game);
      if (game) await startShareWith(game, "smooth");
      else openSharePicker();
    } catch (e) {
      toast(String(e));
    }
  };
  $("home-btn").onclick = enterDmMode;

  $("afk-timeout").addEventListener("change", async e => {
    try {
      state.voice = await invoke("set_afk_timeout", { minutes: Number(e.target.value) });
      const m = state.voice.afk_timeout_minutes;
      toast(m ? `You'll go AFK after ${m === 60 ? "1 hour" : `${m} minute${m === 1 ? "" : "s"}`} of inactivity` : "Automatic AFK turned off");
    } catch (err) {
      toast(String(err));
    }
  });

  $("share-activity-toggle").addEventListener("change", async e => {
    try {
      state.voice = await invoke("set_share_activity", { enabled: e.target.checked });
      toast(e.target.checked ? "Sharing your activity" : "Activity sharing turned off");
    } catch (err) {
      toast(String(err));
    }
  });

  setIcon($("pp-avatar-edit"), "pencil", 16);
  setIcon($("pp-clear"), "close", 14);
  $("pp-avatar-edit").onclick = e => {
    e.stopPropagation();
    openAvatarMenu(e.currentTarget);
  };
  $("pp-emoji").onclick = e => {
    e.stopPropagation();
    openEmojiPop(e.currentTarget, emoji => saveOwnProfile({ statusEmoji: emoji }), {
      custom: true,
      onClear: state.user && state.user.status_emoji ? () => saveOwnProfile({ statusEmoji: "" }) : null,
    });
  };
  $("pp-status").addEventListener("keydown", e => {
    if (e.key === "Enter") e.target.blur();
    if (e.key === "Escape") {
      e.target.value = decodeCustomEmojis(state.user.custom_status);
      e.target.blur();
    }
  });
  $("pp-status").addEventListener("blur", e => {
    const value = e.target.value.trim();
    if (value !== decodeCustomEmojis(state.user.custom_status)) saveOwnProfile({ customStatus: encodeCustomEmojis(value) || null });
  });
  $("pp-clear").onclick = async () => {
    await saveOwnProfile({ customStatus: null, statusEmoji: "" });
    await setStatusExpiry({ status: null });
  };
  for (const btn of $("pp-presence").querySelectorAll("button")) {
    btn.onclick = async () => {
      const presence = btn.dataset.presence;
      if (presence === "online") {
        closeDurations();
        await saveOwnProfile({ presence });
        await setStatusExpiry({ presence: null });
      } else {
        openDurations({ kind: "presence", presence }, btn);
      }
    };
  }
  $("pp-status-expiry").onclick = e => openDurations({ kind: "status" }, e.currentTarget);
  for (const btn of $("pp-durations").querySelectorAll("button[data-mins]")) {
    btn.onclick = () => applyDuration(untilFromChoice(btn.dataset.mins));
  }
  $("pp-custom-set").onclick = () => {
    const when = new Date($("pp-custom-time").value).getTime();
    if (!when || when <= Date.now()) {
      toast("Pick a time in the future");
      return;
    }
    applyDuration(Math.floor(when / 1000));
  };

  $("mc-emoji").replaceChildren(svgIcon("smile", 20));
  wireSuggestions($("mc-input"));
  $("mc-input").addEventListener("keydown", async e => {
    if (e.key !== "Enter" || e.shiftKey) return;
    e.preventDefault();
    const input = e.target;
    const text = input.value.trim();
    if (!text) return;
    const userId = input.dataset.user;
    const name = input.dataset.name;
    input.value = "";
    try {
      await invoke("open_dm", { userId });
      const sent = await invoke("send_dm", { userId, content: encodeCustomEmojis(text) });
      dmSentToast(sent, name, `Sent to ${name}`);
    } catch (err) {
      input.value = text;
      toast(String(err));
    }
  });
  $("mc-emoji").onclick = e => {
    const input = $("mc-input");
    openEmojiPop(e.currentTarget, emoji => {
      const custom = parseEmojiToken(emoji);
      input.value += custom ? `:${custom.name}: ` : emoji;
      input.focus();
    }, { custom: true });
  };
  $("mc-gif").onclick = () => {
    const input = $("mc-input");
    openGifPicker("", { user: input.dataset.user, name: input.dataset.name });
  };
  $("account-pronouns").addEventListener("change", async e => {
    try {
      state.voice = await invoke("set_pronouns", { pronouns: e.target.value });
      e.target.value = state.voice.pronouns || "";
      toast(state.voice.pronouns ? "Pronouns saved" : "Pronouns removed");
    } catch (err) {
      toast(String(err));
    }
  });
  $("account-about").addEventListener("change", async e => {
    try {
      state.user = await invoke("set_about", { about: encodeCustomEmojis(e.target.value) });
      toast("About Me saved");
    } catch (err) {
      toast(String(err));
    }
  });

  $("overlay-enabled").addEventListener("change", e => saveOverlaySettings({ enabled: e.target.checked }));
  $("overlay-corner").addEventListener("click", e => {
    const opt = e.target.closest(".corner-opt");
    if (opt) saveOverlaySettings({ corner: opt.dataset.corner });
  });
  $("overlay-test").addEventListener("click", async () => {
    await saveOverlaySettings({});
    await invoke("preview_overlay");
    setTimeout(() => emitToOverlay("overlay-test", null), 500);
  });
  $("overlay-everyone").addEventListener("change", e => saveOverlaySettings({ show_everyone: e.target.checked }));
  $("overlay-pictures").addEventListener("change", e => saveOverlaySettings({ pictures_only: e.target.checked }));
  $("overlay-opacity").addEventListener("input", e => {
    $("overlay-opacity-label").textContent = `${e.target.value}%`;
  });
  $("overlay-opacity").addEventListener("change", e => saveOverlaySettings({ opacity: Number(e.target.value) / 100 }));
  $("overlay-chat").addEventListener("change", e => saveOverlaySettings({ show_chat: e.target.checked }));
  wireHotkeyInput($("overlay-toggle-key"), combo => saveOverlaySettings({ toggle_hotkey: combo }));
  wireHotkeyInput($("overlay-chat-key"), combo => saveOverlaySettings({ chat_hotkey: combo }));
  wireVoiceInputSettings();
  $("dm-policy").addEventListener("change", saveDmSettings);
  wireProfilePrivacy();
  $("keep-default").addEventListener("change", e => setHistoryWindow(null, e.target.value));
  $("spam-filter-toggle").addEventListener("change", saveDmSettings);
  $("block-cancel").onclick = hideModals;
  $("block-go").onclick = async () => {
    hideModals();
    if (blockTarget) await setBlocked(blockTarget.userId, blockTarget.name, true);
  };
  $("report-cancel").onclick = hideModals;
  $("report-go").onclick = submitReport;
  $("dm-request-accept").onclick = () => answerDmRequest("accept");
  $("dm-request-decline").onclick = () => answerDmRequest("decline");
  $("dm-request-block").onclick = () => answerDmRequest("block");
  $("pp-edit-profile").onclick = () => {
    closeFloating();
    openUserSettings("account");
  };

  $("account-avatar-btn").onclick = e => {
    e.stopPropagation();
    openAvatarMenu(e.currentTarget);
  };
  $("account-banner-upload").onclick = async () => {
    try {
      const src = await chooseImage("Choose a profile banner", $("server-image-file"));
      if (src) await saveAccountBanner(await cropImageFile(src, 600, 240));
    } catch (err) {
      toast(String(err));
    }
  };
  $("account-banner-remove").onclick = () => saveAccountBanner(null);
  $("account-banner-remove").appendChild(svgIcon("close", 14));
  $("account-banner-upload").firstElementChild.appendChild(svgIcon("plus", 14));
  $("account-status-emoji").onclick = e => {
    const set = emoji => {
      accountStatusEmoji = emoji;
      paintEmojiSlot($("account-status-emoji"), emoji);
    };
    openEmojiPop(e.currentTarget, set, { custom: true, onClear: accountStatusEmoji ? () => set("") : null });
  };
  $("account-about-emoji").onclick = e => openEmojiPop(e.currentTarget, picked => {
    insertEmojiInName($("account-about"), picked);
    $("account-about").dispatchEvent(new Event("change", { bubbles: true }));
  }, { custom: true });
  $("avatar-upload").onclick = async () => {
    $("avatar-menu").classList.add("hidden");
    try {
      const src = await chooseImage("Choose your avatar", $("avatar-file"));
      if (src) await applyAvatar("image", await pictureToAvatar(src));
    } catch (err) {
      toast(String(err.message || err));
    }
  };
  $("avatar-emoji").onclick = e => {
    e.stopPropagation();
    $("avatar-menu").classList.add("hidden");
    openEmojiPop(avatarMenuAnchor || e.currentTarget, pickEmojiAvatar, { custom: true, onGif: avatarGifs() });
  };
  $("avatar-remove").onclick = () => {
    $("avatar-menu").classList.add("hidden");
    applyAvatar("none");
  };
  document.addEventListener("mousedown", e => {
    const open = ["emoji-pop", "avatar-menu", "profile-pop", "server-menu", "folder-menu", "channel-menu", "msg-menu", "remind-pop", "member-menu", "pp-durations", "member-card", "invite-menu", "audio-pop", "conn-pop", "fp-menu", "pins-pop", "threads-pop", "composer-menu", "search-pop"].map($).filter(n => !n.classList.contains("hidden"));
    if (!open.length) return;
    if (open.some(n => n.contains(e.target)) || e.target.closest("#user-chip, .caret-btn, .vp-status, #pins-btn, #threads-btn, #attach-btn, #search-box")) return;
    if (e.target.closest("#emoji-pop, #mention-pop, #gif-picker")) return;
    closeFloating();
  });
  window.addEventListener("blur", () => {
    if (!$("remind-pop").contains(document.activeElement)) closeContextMenus();
  });
  document.addEventListener("keydown", e => {
    if (e.key !== "Escape") return;
    closeFloating();
    stopSpeaking();
  });

  setIcon($("reply-bar-cancel"), "close", 16);
  $("reply-bar-cancel").onclick = cancelReply;
  $("awaited-icon").replaceChildren(svgIcon("clock", 16));
  setIcon($("thread-close"), "close", 20);
  $("thread-head-icon").appendChild(svgIcon("thread", 20));
  $("thread-close").onclick = closeThread;
  setIcon($("threads-btn"), "thread", 20);
  $("threads-btn").onclick = toggleThreadsPanel;
  setIcon($("thread-rename"), "pencil", 18);
  setIcon($("thread-archive"), "archive", 18);
  setIcon($("thread-lock"), "lock", 18);
  $("thread-rename").onclick = startThreadRename;
  $("thread-title").onclick = startThreadRename;
  $("thread-archive").onclick = () => toggleThreadFlag("archived");
  $("thread-lock").onclick = () => toggleThreadFlag("locked");
  $("thread-title-input").addEventListener("keydown", e => {
    if (e.key === "Enter") {
      e.preventDefault();
      commitThreadRename();
    }
    if (e.key === "Escape") {
      e.stopPropagation();
      cancelThreadRename();
    }
  });
  $("thread-title-input").addEventListener("blur", () => {
    if (threadRenaming) commitThreadRename();
  });
  $("thread-input").addEventListener("keydown", e => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendThreadReply();
    }
    if (e.key === "Escape") closeThread();
  });
  wireComposerFormatting($("message-input"));
  wireComposerFormatting($("thread-input"));
  $("message-input").addEventListener("keydown", e => {
    if (e.key === "Escape" && state.replyingTo) cancelReply();
  });

  syncGifButton();
  $("gif-btn").onclick = () => ($("gif-picker").classList.contains("hidden") ? openGifPicker("") : closeGifPicker());
  $("gif-search").addEventListener("input", e => {
    clearTimeout(gifSearchTimer);
    gifSearchTimer = setTimeout(() => renderGifResults(e.target.value), 300);
  });
  $("gif-search").addEventListener("keydown", e => {
    if (e.key === "Escape") closeGifPicker();
  });
  for (const tab of $("emoji-pop-tabs").querySelectorAll("button")) tab.onclick = () => showEmojiPopTab(tab.dataset.tab);
  $("emoji-pop-gif-search").addEventListener("input", e => {
    clearTimeout(popGifs.timer);
    popGifs.timer = setTimeout(() => renderGifResults(e.target.value, popGifs), 300);
  });
  document.addEventListener("mousedown", e => {
    if (!$("gif-picker").classList.contains("hidden") && !$("gif-picker").contains(e.target) && !$("gif-btn").contains(e.target)) {
      closeGifPicker();
    }
  });
  const syncGifHelp = () => {
    const intro = serverHasGifs()
      ? "Optional: GIF search works through your Commy account without a key."
      : `${account.server_host || "Your account server"} does not offer GIF search, so add a key of your own to search.`;
    $("gif-key-help").innerHTML = GIF_HELP[$("gif-provider").value] || "";
    $("gif-key-help").prepend(`${intro} `);
    for (const a of $("gif-key-help").querySelectorAll("a[data-href]")) {
      a.onclick = () => invoke("open_url", { url: a.dataset.href }).catch(() => window.open(a.dataset.href, "_blank"));
    }
  };
  $("gif-provider").addEventListener("change", syncGifHelp);
  $("gif-key-save").onclick = () => {
    const settings = { provider: $("gif-provider").value, key: $("gif-key").value.trim() };
    try { localStorage.setItem(GIF_KEY, JSON.stringify(settings)); } catch {}
    toast(settings.key ? "GIF key saved" : "GIF key removed");
    syncGifButton();
  };
  $("gif-key-test").onclick = async () => {
    if (!$("gif-key").value.trim()) return toast("Paste a key first");
    try {
      const gifs = await searchGifs("hello", { provider: $("gif-provider").value, key: $("gif-key").value.trim() });
      toast(gifs.length ? "The key works" : "The key works, but no results came back");
    } catch (e) {
      toast(e.message === "no-key" ? "Paste a key first" : `That key did not work: ${e.message}`);
    }
  };
  const fillGifSettings = () => {
    const s = gifSettings();
    $("gif-provider").value = s.provider;
    $("gif-key").value = s.key;
    syncGifHelp();
  };
  fillGifSettings();

  for (const btn of $("server-menu").querySelectorAll(".ctx-radio")) {
    btn.onclick = () => {
      if (!menuServer) return;
      updateServerPrefs(menuServer.id, { level: btn.dataset.level });
      closeFloating();
      toast(`${menuServer.name}: ${btn.textContent}`);
    };
  }
  $("sm-mute").onclick = () => {
    if (!menuServer) return;
    const muted = !isServerMuted(menuServer.id);
    updateServerPrefs(menuServer.id, { muted });
    closeFloating();
    toast(muted ? `Muted ${menuServer.name}` : `Unmuted ${menuServer.name}`);
  };
  $("sm-notify").onclick = () => {
    const server = menuServer;
    closeFloating();
    if (server) openServerNotify(server);
  };
  for (const btn of $("server-notify-levels").querySelectorAll(".notify-level")) {
    btn.onclick = () => {
      if (!notifyServer) return;
      updateServerPrefs(notifyServer.id, { level: btn.dataset.level });
      renderServerNotify();
    };
  }
  $("server-notify-mute").onchange = e => {
    if (!notifyServer) return;
    updateServerPrefs(notifyServer.id, { muted: e.target.checked });
    renderServerNotify();
  };
  $("server-notify-everyone").onchange = e => setServerFilter("suppressEveryone", !e.target.checked);
  $("server-notify-roles").onchange = e => setServerFilter("suppressRoles", !e.target.checked);
  $("server-notify-done").onclick = hideModals;
  $("sm-read").onclick = () => {
    if (menuServer) markServerRead(menuServer.id);
    closeFloating();
  };
  for (const btn of $("channel-menu").querySelectorAll(".ctx-radio[data-level]")) {
    btn.onclick = () => {
      if (!menuChannel || !state.currentServer) return;
      updateChannelPrefs(state.currentServer.id, menuChannel.id, { level: btn.dataset.level });
      closeFloating();
      toast(`#${menuChannel.name}: ${btn.textContent}`);
    };
  }
  for (const btn of $("channel-menu").querySelectorAll(".ctx-radio[data-mute]")) {
    btn.onclick = () => {
      if (!menuChannel || !state.currentServer) return;
      const choice = MUTE_CHOICES.find(c => c.id === btn.dataset.mute);
      if (!choice) return;
      updateChannelPrefs(state.currentServer.id, menuChannel.id, { mute: muteUntil(choice, nowSecs()) });
      closeFloating();
      toast(`Muted #${menuChannel.name} ${choice.label.replace(/^For /, "for ").replace(/^Until/, "until")}`);
    };
  }
  $("chm-unmute").onclick = () => {
    if (!menuChannel || !state.currentServer) return;
    updateChannelPrefs(state.currentServer.id, menuChannel.id, { mute: false });
    closeFloating();
    toast(`Unmuted #${menuChannel.name}`);
  };
  $("chm-read").onclick = () => {
    if (menuChannel) clearChannelUnread(menuChannel.id);
    closeFloating();
    renderChannels();
    if (menuChannel && state.currentServer) markChannelsRead(state.currentServer.id, [menuChannel.id]);
  };
  $("chm-settings").onclick = () => {
    const channel = menuChannel;
    closeFloating();
    if (channel) openChannelSettings(channel.id);
  };
  $("chm-pin").onclick = () => {
    const channel = menuChannel;
    const server = state.currentServer;
    closeFloating();
    if (channel && server) setChannelPinned(server.id, channel.id, !channelIsPinned(server.id, channel.id));
  };
  const fromMenu = action => async () => {
    closeFloating();
    if (!menuServer) return;
    if (!state.currentServer || state.currentServer.id !== menuServer.id) await selectServer(menuServer.id);
    action();
  };
  $("sm-invite").onclick = fromMenu(() => $("invite-btn").click());
  $("sm-events").onclick = fromMenu(openEvents);
  $("sm-pins").onclick = fromMenu(openServerPins);
  $("sm-rules").onclick = fromMenu(openRules);
  $("sm-roles").onclick = fromMenu(openChooseRoles);
  $("sm-onboarding").onclick = fromMenu(() => openOnboarding("change"));
  $("game-tag-input").addEventListener("keydown", e => {
    if (e.key === "Escape") closeGameSearch();
    if (e.key !== "Enter") return;
    const tag = e.target.value.trim();
    if (!tag) return;
    e.target.value = "";
    closeGameSearch();
    addGameTag(tag, null);
  });
  $("game-tag-input").addEventListener("input", e => queueGameSearch(e.target.value));
  $("game-tag-input").addEventListener("blur", () => setTimeout(closeGameSearch, 150));
  $("igdb-save").onclick = () => saveIgdb(false);
  $("stream-deck-enabled").onchange = e => setStreamDeckEnabled(e.target.checked);
  $("stream-deck-allow").onclick = () => {
    answerStreamDeckPairing(true);
    hideModals();
  };
  $("stream-deck-deny").onclick = hideModals;
  $("igdb-remove").onclick = () => saveIgdb(true);
  for (const link of document.querySelectorAll(".integration-steps a[data-href]")) {
    wireExternalLink(link, link.dataset.href);
  }
  $("rules-input").addEventListener("input", syncRulesPreview);
  $("rules-save").onclick = () => saveRules(false);
  $("rules-remove").onclick = () => saveRules(true);
  $("automod-terms").addEventListener("input", syncAutomodCount);
  $("automod-save").onclick = saveAutomod;
  $("rules-close").onclick = hideModals;
  $("rules-accept").onclick = acceptRules;
  $("rules-gate-open").onclick = () => openServerWelcome("rules");
  $("sw-later").onclick = hideModals;
  $("ob-later").onclick = hideModals;
  $("ob-back").onclick = () => turnOnboardingPage(-1);
  $("ob-next").onclick = advanceOnboarding;
  $("ob-add-question").onclick = addOnboardingQuestion;
  $("ob-save").onclick = () => saveOnboarding(false);
  $("ob-remove").onclick = () => saveOnboarding(true);
  $("ob-preview").onclick = previewOnboarding;
  $("si-answers").onclick = () => openOnboarding("change");
  $("events-close").replaceChildren(svgIcon("close", 18));
  $("events-close").onclick = closeEvents;
  $("event-create").onclick = () => openEventEditor(null);
  for (const tab of document.querySelectorAll(".events-tab")) {
    tab.onclick = () => {
      eventsView.mode = tab.dataset.view;
      renderEvents();
    };
  }
  $("events-layer").addEventListener("mousedown", e => {
    if (e.target === e.currentTarget) closeEvents();
  });
  $("event-cancel").onclick = hideModals;
  $("event-save").onclick = saveEvent;
  $("choose-roles-done").onclick = hideModals;
  $("create-category-icon").replaceChildren(svgIcon("chevronDown", 20));
  $("sm-settings").onclick = fromMenu(() => openServerSettings());
  $("sm-ungroup").onclick = () => {
    closeFloating();
    if (!menuServer) return;
    const layout = removeFromLayout(loadRailLayout(), menuServer.id);
    layout.push({ type: "server", id: menuServer.id });
    saveRailLayout(normalizeRailLayout(layout, railServers().map(s => s.id)));
    renderServerRail();
  };
  $("sm-leave").onclick = () => {
    closeFloating();
    if (menuServer) openLeaveServer(menuServer.id);
  };
  $("leave-server-go").onclick = confirmLeaveServer;
  $("leave-server-cancel").onclick = hideModals;
  $("delete-server-go").onclick = confirmDeleteServer;
  $("delete-server-cancel").onclick = hideModals;
  $("delete-server-input").addEventListener("keydown", e => {
    if (e.key === "Enter" && !$("delete-server-go").disabled) confirmDeleteServer();
  });
  $("server-delete-open").onclick = () => state.currentServer && openDeleteServer(state.currentServer);
  $("server-transfer-open").onclick = () => state.currentServer && openTransferOwner(state.currentServer);
  $("transfer-owner-cancel").onclick = hideModals;
  $("transfer-owner-go").onclick = confirmTransferOwner;
  $("transfer-owner-input").onkeydown = e => {
    if (e.key === "Enter" && !$("transfer-owner-go").disabled) confirmTransferOwner();
  };
  $("audit-group").onchange = paintAuditLog;
  $("audit-actor").onchange = paintAuditLog;
  $("fm-ungroup").onclick = () => {
    closeFloating();
    if (!menuFolder) return;
    const layout = [];
    for (const item of loadRailLayout()) {
      if (item.id === menuFolder.id) for (const id of item.servers) layout.push({ type: "server", id });
      else layout.push(item);
    }
    saveRailLayout(layout);
    renderServerRail();
  };
  $("fm-rename").onclick = () => {
    closeFloating();
    if (!menuFolder) return;
    $("rename-folder-input").value = menuFolder.name || "";
    showModal("modal-rename-folder");
    $("rename-folder-input").focus();
  };
  const saveFolderName = () => {
    const name = $("rename-folder-input").value.trim().slice(0, 32);
    saveRailLayout(loadRailLayout().map(i => (menuFolder && i.id === menuFolder.id ? { ...i, name } : i)));
    hideModals();
    renderServerRail();
  };
  $("rename-folder-save").onclick = saveFolderName;
  $("rename-folder-input").addEventListener("keydown", e => {
    if (e.key === "Enter") saveFolderName();
  });
  $("rename-folder-cancel").onclick = hideModals;
  wireNicknameModal();
  wireVoiceStatus();

  const uploadServerImage = async which => {
    try {
      const src = await chooseImage(which === "icon" ? "Choose a server avatar" : "Choose a server banner", $("server-image-file"));
      if (!src) return;
      const data = which === "icon" ? await cropImageFile(src, 256, 256) : await cropImageFile(src, 960, 320);
      await setServerImage(which, data);
    } catch (err) {
      toast(String(err));
    }
  };
  $("overview-icon").onclick = () => uploadServerImage("icon");
  $("banner-upload").onclick = () => uploadServerImage("banner");
  $("overview-icon-remove").onclick = () => setServerImage("icon", null);
  $("banner-remove").onclick = () => setServerImage("banner", null);
  $("invite-bg-upload").onclick = uploadInviteBackground;
  $("invite-bg-remove").onclick = () => setServerImage("invite", null);
  $("invite-custom-name").addEventListener("input", paintInviteNameCheck);
  $("invite-custom-name").addEventListener("keydown", e => {
    if (e.key === "Enter") createCustomInvite();
  });
  $("invite-custom-create").onclick = createCustomInvite;

  $("notify-level").addEventListener("change", e => {
    const prefs = notifyPrefs();
    prefs.level = e.target.value;
    saveNotifyPrefs(prefs);
  });
  $("friend-online-toggle").addEventListener("change", e => {
    const prefs = notifyPrefs();
    prefs.friendOnline = e.target.checked;
    saveNotifyPrefs(prefs);
  });
  $("tts-mode").addEventListener("change", e => {
    saveTtsPrefs({ ...ttsPrefs(), mode: e.target.value });
    renderTtsSettings();
  });
  $("tts-rate").addEventListener("input", e => {
    saveTtsPrefs({ ...ttsPrefs(), rate: clampRate(Number(e.target.value) / 10) });
    renderTtsSettings();
  });
  $("tts-pitch").addEventListener("input", e => {
    saveTtsPrefs({ ...ttsPrefs(), pitch: clampPitch(Number(e.target.value) / 10) });
    renderTtsSettings();
  });
  $("tts-voice").addEventListener("change", e => {
    saveTtsPrefs({ ...ttsPrefs(), voice: e.target.value || null });
    renderTtsSettings();
  });
  $("tts-preview").onclick = () => {
    if (!speakText(spokenText(state.user ? state.user.display_name || state.user.username : "Commy", "this is how text-to-speech sounds"))) toast("Text-to-speech is not available here");
  };
  $("close-to-tray-toggle").addEventListener("change", async e => {
    try {
      state.voice = await invoke("set_close_to_tray", { enabled: e.target.checked });
    } catch (err) {
      toast(String(err));
    }
  });
  $("autostart-toggle").addEventListener("change", async e => {
    try {
      e.target.checked = await invoke("set_autostart", { enabled: e.target.checked });
    } catch (err) {
      e.target.checked = !e.target.checked;
      toast(String(err));
    }
  });
  $("auto-update-toggle").addEventListener("change", async e => {
    try {
      state.voice = await invoke("set_auto_update", { enabled: e.target.checked });
    } catch (err) {
      toast(String(err));
    }
    renderUpdateStatus();
  });
  $("update-action").onclick = runUpdateAction;
  $("conn-test-run").onclick = runConnectionTest;
  $("verbose-log-toggle").addEventListener("change", e => setVerboseLogging(e.target.checked));
  $("log-folder-open").onclick = () => invoke("open_log_folder").catch(err => toast(String(err)));
  $("log-bundle-save").onclick = saveLogBundle;
  $("sounds-toggle").addEventListener("change", e => {
    const prefs = soundPrefs();
    prefs.enabled = e.target.checked;
    saveSoundPrefs(prefs);
  });
  $("sound-volume").addEventListener("input", e => {
    $("sound-volume-label").textContent = `${e.target.value}%`;
  });
  $("sound-volume").addEventListener("change", e => {
    const prefs = soundPrefs();
    prefs.volume = Number(e.target.value) / 100;
    saveSoundPrefs(prefs);
    playSound("message", soundFor("message") === "none" ? "notification" : soundFor("message"));
  });
  $("notify-test").onclick = () =>
    invoke("notify", { title: "Commy", body: "Notifications are working." }).then(() => playSound("message")).catch(e => toast(String(e)));
  window.addEventListener("focus", () => {
    if (state.view === "server" && state.currentChannel) {
      if (readState.held === state.currentChannel.id) return;
      clearChannelUnread(state.currentChannel.id);
      if (!state.historyAnchor) advanceRead(state.currentChannel.id, state.renderedMessages || []);
    }
    if (state.view === "dm" && state.currentDm) clearDmUnread(state.currentDm.user);
  });

  $("vp-camera").onclick = () => (state.cameraOn ? stopCamera() : startCamera());
  $("cam-preview-btn").onclick = toggleCameraPreview;
  for (const button of $("cam-bg").querySelectorAll("button")) {
    button.onclick = () => {
      if (button.dataset.bg === "image" && !hasCameraBackdrop()) chooseCameraBackdrop();
      else updateCameraFx({ background: button.dataset.bg });
    };
  }
  $("cam-backdrop-btn").onclick = chooseCameraBackdrop;
  for (const button of $("cam-avatar").querySelectorAll("button")) {
    button.onclick = () => pickCameraAvatar(button.dataset.avatar);
  }
  $("cam-avatar-btn").onclick = chooseCameraAvatar;
  $("cam-mirror").addEventListener("change", e => updateCameraFx({ mirror: e.target.checked }));
  onCameraFxChange(renderCameraFx);
  renderCameraFx();
  $("camera-device").addEventListener("change", async e => {
    try { localStorage.setItem(CAMERA_KEY, e.target.value); } catch {}
    if (previewPipe) {
      stopCameraPreview();
      toggleCameraPreview();
    }
    if (state.cameraOn) {
      await stopCamera();
      await startCamera();
    }
  });
  $("input-device").addEventListener("change", saveAudioDevices);
  $("output-device").addEventListener("change", saveAudioDevices);

  wireSuggestions($("message-input"), { commands: true });
  wireSuggestions($("thread-input"));

  $("delete-message-go").onclick = () => pendingDelete && deleteMessage(pendingDelete);
  $("forward-cancel").onclick = hideModals;
  $("forward-search").addEventListener("input", renderForwardList);
  $("delete-message-cancel").onclick = hideModals;
  $("message-input").addEventListener("keydown", e => {
    if (e.key !== "ArrowUp" || e.target.value || state.view !== "server") return;
    const mine = (state.renderedMessages || []).filter(m => isOwnMessage(m) && m.content && !parseStickerToken(m.content) && !isHidden(m.id));
    if (!mine.length) return;
    e.preventDefault();
    startEditMessage(mine[mine.length - 1].id);
  });

  $("channel-edit-name").addEventListener("input", syncChannelDirty);
  $("channel-edit-color").addEventListener("input", e => {
    channelColorDraft = e.target.value.replace(/^#/, "").toLowerCase();
    paintChannelBandPreview();
    syncChannelDirty();
  });
  $("channel-edit-color-clear").onclick = () => {
    channelColorDraft = null;
    paintChannelBandPreview();
    syncChannelDirty();
  };
  $("channel-edit-emoji").onclick = e => pickChannelLead(e.currentTarget, "edit", syncChannelDirty);
  $("channel-banner-upload").onclick = async () => {
    try {
      const src = await chooseImage("Choose a channel banner", $("server-image-file"));
      if (!src) return;
      await setChannelBanner(await cropImageFile(src, 960, 320));
    } catch (err) {
      toast(String(err));
    }
  };
  $("channel-banner-remove").onclick = () => setChannelBanner(null);
  $("channel-edit-name").addEventListener("keydown", e => {
    if (e.key === "Enter") saveChannelEdits();
  });
  $("channel-edit-retention").addEventListener("change", syncChannelDirty);
  $("channel-slow").addEventListener("change", syncChannelDirty);
  $("channel-limit").addEventListener("input", () => {
    syncLimitLabel();
    syncChannelDirty();
  });
  $("channel-edit-save").onclick = saveChannelEdits;
  $("channel-edit-announce").onchange = async e => {
    const channel = editingChannel();
    if (!channel || !state.currentServer) return;
    const kind = e.target.checked ? "announcement" : "text";
    try {
      await invoke("set_channel_kind", { serverId: state.currentServer.id, channelId: channel.id, kind });
      toast(kind === "announcement" ? "Now an announcement channel" : "Now a text channel");
      await refreshCurrentServer();
      openChannelSettings(channel.id);
    } catch (err) {
      e.target.checked = !e.target.checked;
      toast(String(err));
    }
  };
  $("channel-edit-sensitive").onchange = async e => {
    const channel = editingChannel();
    if (!channel || !state.currentServer) return;
    const sensitive = e.target.checked;
    try {
      await invoke("set_channel_sensitive", { serverId: state.currentServer.id, channelId: channel.id, sensitive });
      toast(sensitive ? "Channel is now age-restricted" : "Age restriction removed");
      await refreshCurrentServer();
      openChannelSettings(channel.id);
    } catch (err) {
      e.target.checked = !e.target.checked;
      toast(String(err));
    }
  };
  $("sensitive-reset").onclick = resetSensitiveConfirmations;
  $("trusted-links-forget").onclick = () => saveTrustedLinkDomains([]);
  $("leaving-cancel").onclick = closeLeavingPrompt;
  $("leaving-go").onclick = confirmLeaving;
  $("pins-btn").onclick = togglePinsPanel;
  wireSearch();
  wireSwitcher();
  $("pins-all").onclick = openServerPins;
  $("pins-modal-close").onclick = hideModals;
  $("announce-gate-icon").appendChild(svgIcon("megaphone", 22));
  $("channel-edit-reset").onclick = () => openChannelSettings(editingChannelId);
  $("channel-access-save").onclick = saveChannelAccess;
  $("channel-access-reset").onclick = () => openChannelSettings(editingChannelId, "permissions");
  $("channel-settings-close").onclick = closeSettingsLayers;
  $("channel-delete-btn").onclick = openDeleteChannel;
  $("channel-delete-nav").onclick = openDeleteChannel;
  $("delete-channel-confirm").addEventListener("input", e => {
    const channel = editingChannel();
    $("delete-channel-go").disabled = !channel || e.target.value.trim() !== channel.name;
  });
  $("delete-channel-confirm").addEventListener("keydown", e => {
    if (e.key === "Enter" && !$("delete-channel-go").disabled) confirmDeleteChannel();
  });
  $("delete-channel-go").onclick = confirmDeleteChannel;
  $("delete-channel-cancel").onclick = hideModals;

  for (const opt of $("create-channel-kinds").querySelectorAll(".kind-option")) {
    opt.onclick = () => setCreateChannelKind(opt.dataset.kind);
  }
  $("create-voice-icon").appendChild(svgIcon("speaker", 22));
  $("create-stage-icon").appendChild(svgIcon("stage", 22));
  $("create-announcement-icon").appendChild(svgIcon("megaphone", 22));
  $("create-forum-icon").appendChild(svgIcon("forum", 22));
  wireForum();
  $("create-channel-name").addEventListener("input", syncCreateChannelName);
  $("create-channel-emoji").onclick = e => pickChannelLead(e.currentTarget, "create", syncCreateChannelName);
  $("create-channel-name").addEventListener("keydown", e => {
    if (e.key === "Enter" && !$("create-channel-save").disabled) saveCreateChannel();
  });
  $("create-channel-save").onclick = saveCreateChannel;
  $("create-channel-cancel").onclick = hideModals;

  $("vp-stop-share").onclick = toggleShare;
  $("vp-change-source").onclick = openSharePicker;
  $("vp-stop-watch").onclick = stopWatching;
  $("vp-disconnect").onclick = disconnectVoice;
  document.querySelector(".vp-status").title = "Connection details";
  document.querySelector(".vp-status").addEventListener("click", e => {
    e.stopPropagation();
    if ($("conn-pop").classList.contains("hidden")) openConnPop();
    else closeConnPop();
  });
  $("vp-conn-meta").onclick = () => {
    if (!state.voiceServer) return;
    if (state.voiceServer.call) return messageUser(state.voiceServer.call, state.voiceServer.name);
    if (!state.currentServer || state.currentServer.id !== state.voiceServer.id) {
      selectServer(state.voiceServer.id);
    }
  };
  $("dm-call-btn").onclick = () => (isGroupView() ? toggleGroupCall() : startDmCall());
  $("group-add-btn").onclick = () => {
    if (isGroupView()) openGroupModal(state.currentServer.id);
    else if (inDmView()) openGroupModal(null, [state.currentDm.user]);
  };
  $("group-leave-btn").onclick = () => {
    if (isGroupView()) openLeaveServer(state.currentServer.id);
  };
  $("group-cancel").onclick = hideModals;
  $("group-go").onclick = confirmGroupModal;
  $("group-search").addEventListener("input", renderGroupList);
  $("call-accept").onclick = () => answerDmCall(true);
  $("call-decline").onclick = () => answerDmCall(false);
  setIcon($("group-add-btn"), "friendAdd");
  setIcon($("call-accept"), "phone", 26);
  setIcon($("call-decline"), "hangup", 26);
  $("ap-nc").onchange = async e => {
    try {
      state.voice = await invoke("set_noise_cancellation", { enabled: e.target.checked });
      $("nc-toggle").checked = state.voice.noise_cancellation;
      toast(`Noise cancellation ${state.voice.noise_cancellation ? "on" : "off"}`);
    } catch (err) {
      e.target.checked = Boolean(state.voice.noise_cancellation);
      toast(String(err));
    }
  };

  $("dm-search").addEventListener("input", () => renderDmThreads());
  $("dm-search").addEventListener("keydown", e => {
    if (e.key === "Escape") {
      e.target.value = "";
      renderDmThreads();
    } else if (e.key === "Enter") {
      const first = $("channel-list").querySelector(".dm-thread");
      if (first) first.click();
      e.target.value = "";
    }
  });
  $("members-btn").onclick = () => {
    if (phoneStep("people")) return;
    const collapsed = membersCollapsed(readMembersPref(), window.innerWidth);
    try {
      localStorage.setItem("commy.members.collapsed", collapsed ? "0" : "1");
    } catch {}
    applyMembersLayout();
  };
  window.addEventListener("resize", applyMembersLayout);
  applyMembersLayout();
  setupSidebarResize();
  $("dm-profile-btn").onclick = () => {
    if (isPhoneWidth(window.innerWidth)) {
      state.showDmProfile = true;
      if (state.currentDm) renderDmProfile(state.currentDm.user, state.currentDm.name);
      phoneStep("people");
      return;
    }
    state.showDmProfile = !state.showDmProfile;
    if (state.currentDm) renderDmProfile(state.currentDm.user, state.currentDm.name);
  };
  $("new-dm-search").addEventListener("input", renderNewDmList);
  $("new-dm-search").addEventListener("keydown", e => {
    if (e.key === "Enter") {
      const first = $("new-dm-list").querySelector(".person-row");
      if (first) first.click();
    }
  });

  $("lightbox").onclick = closeLightbox;
  window.addEventListener("keydown", e => {
    if (e.key !== "Escape") return;
    if (switcher.open) closeSwitcher();
    else if (!$("lightbox").classList.contains("hidden")) closeLightbox();
    else if (!$("modal-backdrop").classList.contains("hidden") && state.user) hideModals();
    else if (openSettingsLayerId()) closeSettingsLayers();
  });

  $("message-input").addEventListener("keydown", e => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  });

  $("message-input").addEventListener("input", e => {
    scheduleLinkDraft(e.inputType === "insertFromPaste" ? 0 : LINK_DRAFT_DELAY_MS);
    noteTyped(e.target);
  });
  $("thread-input").addEventListener("input", e => noteTyped(e.target));
  setIcon($("link-draft-dismiss"), "close", 16);
  $("link-draft-dismiss").onclick = dismissLinkDraft;
  setIcon($("lightbox-open"), "openExternal", 16);
  $("lightbox-open").append("Open original");
  setIcon($("lightbox-save"), "download", 16);
  $("lightbox-save").append("Save");
  renderLinkPreviewSettings();
  renderTrustedLinkSettings();
  $("link-preview-send-toggle").addEventListener("change", onLinkPreviewSettingsChange);
  $("link-preview-show-toggle").addEventListener("change", onLinkPreviewSettingsChange);

  $("message-input").addEventListener("paste", e => {
    const files = Array.from(e.clipboardData?.files || []);
    if (!files.length) return;
    e.preventDefault();
    const src = imageSourceFromHtml(e.clipboardData.getData("text/html"));
    if (src) {
      const input = $("message-input");
      input.value = (input.value ? input.value.trimEnd() + " " : "") + src;
      return;
    }
    for (const file of files) uploadFileObject(file);
  });

  const messagesBox = $("messages");
  for (const evt of ["dragover", "dragenter"]) {
    messagesBox.addEventListener(evt, e => {
      e.preventDefault();
      messagesBox.classList.add("drop-hint");
    });
  }
  messagesBox.addEventListener("dragleave", () => messagesBox.classList.remove("drop-hint"));
  messagesBox.addEventListener("drop", e => {
    e.preventDefault();
    messagesBox.classList.remove("drop-hint");
    for (const file of Array.from(e.dataTransfer?.files || [])) uploadFileObject(file);
  });

  $("share-cancel").onclick = () => {
    sharePicker.token++;
    hideModals();
  };

  const volLabel = () => {
    $("output-vol-label").textContent = `${$("output-vol").value}%`;
    $("input-gain-label").textContent = `${$("input-gain").value}%`;
  };
  for (const id of ["output-vol", "input-gain"]) {
    $(id).addEventListener("input", volLabel);
    $(id).addEventListener("change", async () => {
      try {
        state.voice = await invoke("set_volumes", {
          outputVolume: Number($("output-vol").value) / 100,
          inputGain: Number($("input-gain").value) / 100,
        });
      } catch (e) {
        toast(String(e));
      }
    });
  }

  $("modal-backdrop").addEventListener("mousedown", e => {
    if (e.target === e.currentTarget && state.user && account.signed_in) hideModals();
  });

  if (IS_TAURI) {
    window.__TAURI__.event.listen("server-event", async event => {
      const serverId = event.payload;
      loadShareableEmojis();
      await refreshServersIfChanged();
      if (state.currentServer && serverId === state.currentServer.id) {
        await refreshCurrentServer();
      }
      if (state.view === "dm") {
        await refreshPeople();
        await renderDmThreads();
        if (state.currentDm) renderDmHeader(state.currentDm.user, state.currentDm.name);
      }
      await trackFriendPresence(serverId);
    });
    window.__TAURI__.event.listen("servers-changed", refreshServers);
    window.__TAURI__.event.listen("soundboard-play", event => onSoundboardPlay(event.payload));
    window.__TAURI__.event.listen("activity-set", event => onActivitySet(event.payload));
    window.__TAURI__.event.listen("activity-input", event => onActivityInput(event.payload));
    window.__TAURI__.event.listen("stream-deck-pair", event => onStreamDeckPair(event.payload));
    window.__TAURI__.event.listen("stream-deck-pair-closed", event => onStreamDeckPairClosed(event.payload));
    window.__TAURI__.event.listen("stream-deck-command", event => onStreamDeckCommand(event.payload));
    window.__TAURI__.event.listen("voice-settings-synced", event => {
      state.voice = event.payload;
      renderUserPanel();
    });
    window.__TAURI__.event.listen("stream-deck-changed", event => {
      streamDeck = event.payload;
      renderStreamDeck();
    });
    window.__TAURI__.event.listen("report-received", event => onReportReceived(event.payload));
    window.__TAURI__.event.listen("account-changed", onAccountChanged);
    window.__TAURI__.event.listen("sign-in-failed", event => showSignInError(String(event.payload)));
    window.__TAURI__.event.listen("friends-changed", async event => {
      await loadFriends();
      const p = event.payload || {};
      if (p.signal === "request" && friendState(p.user) === "incoming") {
        toast(`${p.name} sent you a friend request`);
        const body = p.note ? `${p.name}: ${p.note}` : `${p.name} wants to be friends`;
        invoke("notify", { title: "Friend request", body, target: `dm:${p.user}` }).catch(() => {});
        playSound("message");
      } else if (p.signal === "accept" && friendState(p.user) === "friend") {
        toast(`You and ${p.name} are now friends`);
      }
      if (state.view === "dm") renderDmThreads();
    });
    window.__TAURI__.event.listen("voice-moderated", event => {
      const { muted, deafened } = event.payload;
      toast(deafened ? "A moderator server-deafened you" : muted ? "A moderator server-muted you" : "A moderator lifted your server mute");
      document.body.classList.toggle("server-muted", muted || deafened);
      renderChannels();
    });
    window.__TAURI__.event.listen("stage-role", event => {
      const { audience } = event.payload || {};
      toast(audience ? "You are in the audience now" : "You can speak now");
      renderVoicePanel();
      renderChannels();
    });
    window.__TAURI__.event.listen("timed-out", event => {
      const { until } = event.payload || {};
      toast(timeoutActive(until) ? `A moderator timed you out for ${timeoutLeftText(until)}` : "A moderator removed your timeout");
    });
    window.__TAURI__.event.listen("owner-changed", event => {
      const { name, who, mine } = event.payload;
      toast(mine ? `You now own ${name || "a server"}` : `${who || "Someone else"} now owns ${name || "a server"}`);
    });
    window.__TAURI__.event.listen("server-deleted", async event => {
      toast(`${event.payload.name || "A server"} was deleted by its owner`);
      await leaveServerView(event.payload.server);
    });
    const removedFrom = (verb) => async event => {
      toast(`You were ${verb} ${event.payload.name || "a server"}`);
      if (state.voiceServer && state.voiceServer.id === event.payload.server) {
        state.voiceChannelId = null;
        state.voiceServer = null;
        state.sharing = false;
      }
      await refreshServers();
      if (state.currentServer && state.currentServer.id === event.payload.server) {
        state.currentServer = null;
        await enterDmMode();
      }
    };
    window.__TAURI__.event.listen("banned", removedFrom("banned from"));
    window.__TAURI__.event.listen("kicked", removedFrom("removed from"));
    window.__TAURI__.event.listen("voice-left", async event => {
      if (state.voiceServer && state.voiceServer.id === event.payload) {
        state.voiceChannelId = null;
        state.voiceServer = null;
        state.sharing = false;
        toast("You left voice because that channel is now locked to other roles");
      }
      await refreshCurrentServer();
    });
    window.__TAURI__.event.listen("overlay-hello", () => {
      lastOverlayRoster = "";
      pushOverlayRoster(true);
    });
    window.__TAURI__.event.listen("update-status", event => onUpdateStatus(event.payload));
    window.__TAURI__.event.listen("overlay-settings-changed", event => {
      overlaySettings = event.payload;
      renderOverlaySettings();
    });
    window.__TAURI__.event.listen("message-created", event => {
      const msg = event.payload;
      if (state.currentChannel && msg.channel === state.currentChannel.id && !isBlocked(msg.author)) {
        pushOverlayChat(msg.author, msg.author_name, msg.content, msg.has_attachments);
      }
    });
    window.__TAURI__.event.listen("event-reminder", () => playSound("message"));
    window.__TAURI__.event.listen("safety-changed", async () => {
      await loadSafety();
      afterSafetyChange();
    });
    window.__TAURI__.event.listen("dm-changed", async event => {
      if (state.view === "dm") {
        if (state.currentDm && event.payload === state.currentDm.user) {
          await selectDm(state.currentDm.user, state.currentDm.name);
        } else {
          await renderDmThreads();
        }
      } else {
        toast("New direct message");
      }
    });
    for (const name of ["dm-sending", "dm-sent"]) {
      window.__TAURI__.event.listen(name, async event => {
        if (state.view !== "dm") return;
        if (state.currentDm && event.payload === state.currentDm.user) {
          await selectDm(state.currentDm.user, state.currentDm.name);
        } else {
          await renderDmThreads();
        }
      });
    }
    window.__TAURI__.event.listen("afk-disconnected", async () => {
      toast("You were disconnected from voice for inactivity");
      state.voiceChannelId = null;
      state.voiceServer = null;
      state.sharing = false;
      clearSpeaking();
      playSound("leave");
      renderVoicePanel();
      if (state.currentServer) await refreshCurrentServer();
    });
    window.__TAURI__.event.listen("voice-elsewhere", async () => {
      toast("Your voice moved to your other device");
      state.voiceChannelId = null;
      state.voiceServer = null;
      state.sharing = false;
      stopCameraTracks();
      state.cameraOn = false;
      cam.watching.clear();
      clearSpeaking();
      playSound("leave");
      renderVoicePanel();
      if (state.currentServer) await refreshCurrentServer();
    });
    window.__TAURI__.event.listen("voice-moved", async event => {
      const { server, channel, name } = event.payload;
      if (!channel) {
        toast("A moderator disconnected you from voice");
        state.voiceChannelId = null;
        state.voiceServer = null;
        state.sharing = false;
        clearSpeaking();
        playSound("leave");
      } else {
        toast(`A moderator moved you to ${name || "another channel"}`);
        state.voiceChannelId = channel;
        if (state.voiceServer && state.voiceServer.id === server && name) state.voiceServer.channelName = name;
      }
      renderVoicePanel();
      if (state.currentServer && state.currentServer.id === server) await refreshCurrentServer();
    });
    window.__TAURI__.event.listen("afk-moved", async event => {
      const { server, channel, name } = event.payload || {};
      toast("You were moved to AFK for inactivity");
      if (channel) state.voiceChannelId = channel;
      if (name && state.voiceServer && state.voiceServer.id === server) state.voiceServer.channelName = name;
      renderVoicePanel();
      if (state.currentServer && state.currentServer.id === server) await refreshCurrentServer();
    });
    window.__TAURI__.event.listen("profile-changed", event => {
      state.user = event.payload;
      renderUserPanel();
      renderVoicePanel();
      invoke("get_voice_settings").then(v => {
        state.voice = v;
        if (!$("profile-pop").classList.contains("hidden")) renderProfilePopover();
      }).catch(() => {});
    });
    window.__TAURI__.event.listen("camera-frame", event => showCameraFrame(event.payload));
    window.__TAURI__.event.listen("message-created", event => onMessageCreated(event.payload));
    window.__TAURI__.event.listen("typing", event => onTyping(event.payload));
    window.__TAURI__.event.listen("messages-caught-up", event => onMessagesCaughtUp(event.payload));
    window.__TAURI__.event.listen("read-state-changed", event => applyReadView(event.payload));
    window.__TAURI__.event.listen("sync-prefs-changed", event => applySyncedPrefs(event.payload));
    window.__TAURI__.event.listen("saved-changed", event => applySavedStore(event.payload));
    loadSyncedPrefs();
    window.__TAURI__.event.listen("dm-received", event => onDmReceived(event.payload));
    window.__TAURI__.event.listen("dm-call", event => onDmCall(event.payload));
    invoke("get_dm_call").then(call => call && onDmCall(call)).catch(() => {});
    window.__TAURI__.event.listen("notification-clicked", event => openNotificationTarget(event.payload));
    window.__TAURI__.event.listen("tray-toggle-mute", () => $("mute-btn").click());
    window.__TAURI__.event.listen("voice-settings-changed", event => {
      state.voice = event.payload;
      renderUserPanel();
      toast(state.voice.deafened ? "Deafened" : state.voice.muted ? "Muted" : "Unmuted");
    });
    window.__TAURI__.event.listen("talk-key", event => setTalking(Boolean(event.payload)));
    window.__TAURI__.event.listen("voice-speaking", event => {
      setSpeaking(event.payload.user, Boolean(event.payload.speaking));
    });
    window.__TAURI__.event.listen("video-frame", event => {
      const frame = event.payload;
      if (state.watchingUser && frame.user === state.watchingUser) {
        showStageFrame(`data:image/jpeg;base64,${frame.jpeg}`, frame.width, frame.height);
      } else if (state.sharing && state.user && frame.user === state.user.id) {
        showOwnFrame(frame);
      }
    });
  }
}

async function stopMicTest() {
  try { await invoke("stop_mic_test"); } catch (e) {}
  state.micTesting = false;
  $("mic-test-btn").textContent = "Start Test";
}

const EXPRESSION_KINDS = {
  emojis: { one: "an emoji", many: "emojis", search: "Search emojis" },
  stickers: { one: "a sticker", many: "stickers", search: "Search stickers" },
  emoticons: { one: "an emoticon", many: "emoticons", search: "Search emoticons" },
};
let pickerKind = "emojis";

function composerEmojiBrowser() {
  if (!emojiBrowsers.composer) emojiBrowsers.composer = buildEmojiBrowser($("composer-emoji-browser"));
  return emojiBrowsers.composer;
}

function insertPickedIntoComposer(value) {
  const custom = parseEmojiToken(value);
  insertIntoComposer(custom ? `:${custom.name}: ` : value);
}

function setPickerKind(kind) {
  pickerKind = kind;
  for (const tab of $("picker-tabs").querySelectorAll("button")) {
    const on = tab.dataset.kind === kind;
    tab.classList.toggle("active", on);
    tab.setAttribute("aria-selected", String(on));
  }
  const emojis = kind === "emojis";
  $("composer-emoji-browser").classList.toggle("hidden", !emojis);
  $("picker-plain").classList.toggle("hidden", emojis);
  const typing = !isPhoneWidth(window.innerWidth);
  if (emojis) {
    const b = composerEmojiBrowser();
    openEmojiBrowser(b, { custom: true, composer: true, onPick: insertPickedIntoComposer });
    if (typing) b.search.focus();
    return;
  }
  $("emoji-search").placeholder = EXPRESSION_KINDS[kind].search;
  $("emoji-search").value = "";
  renderEmojiGrid("");
  if (typing) $("emoji-search").focus();
}

function renderEmojiGrid(query) {
  const grid = $("emoji-grid");
  grid.replaceChildren();
  grid.dataset.kind = pickerKind;
  grid.scrollTop = 0;
  const q = query.trim().toLowerCase();
  if (pickerKind === "stickers") renderStickerTab(grid, q);
  else renderEmoticonTab(grid, q);
  if (!grid.childElementCount) grid.appendChild(el("div", "picker-empty", "No matches"));
}

function renderStickerTab(grid, q) {
  const any = appendExpressionGroups(grid, "stickers", q, sticker => {
    const btn = el("button", "sticker-tile");
    btn.title = sticker.name;
    btn.appendChild(customEmojiImg(sticker.name, sticker.image, "sticker-img"));
    btn.onclick = () => sendSticker(sticker);
    return btn;
  });
  if (!any && !q && !grid.childElementCount) {
    grid.appendChild(el("div", "picker-empty", "No stickers here yet. Servers add them in Server Settings, then Stickers."));
  }
  appendCatalogSection(grid, q, account.server_stickers ? "klipy" : "stickers");
}

function renderEmoticonTab(grid, q) {
  const face = item => {
    const btn = el("button", "emoticon-tile");
    btn.title = `:${item.name}:`;
    btn.appendChild(el("span", "emoticon-text", item.text));
    btn.appendChild(el("span", "emoticon-name", item.name.replace(/_/g, " ")));
    btn.onclick = () => insertIntoComposer(`${item.text} `);
    return btn;
  };
  appendExpressionGroups(grid, "emoticons", q, face);
  for (const { group, faces } of BUILTIN_EMOTICONS) {
    const matches = expressionMatches(faces.map(([name, text]) => ({ name, text })), q, "text");
    if (!matches.length) continue;
    pickerSection(grid, group);
    for (const item of matches) grid.appendChild(face(item));
  }
}

async function sendSticker(sticker) {
  $("emoji-picker").classList.add("hidden");
  const content = stickerToken(sticker.name, sticker.image);
  if (state.view === "dm" && state.currentDm) {
    const { user, name } = state.currentDm;
    try {
      await invoke("send_dm", { userId: user, content });
      await selectDm(user, name);
    } catch (e) {
      toast(String(e));
    }
    return;
  }
  if (!state.currentServer || !state.currentChannel) return;
  const replyTo = state.replyingTo ? state.replyingTo.id : null;
  $("messages").dataset.view = "";
  cancelReply();
  try {
    await invoke("send_message", { serverId: state.currentServer.id, channelId: state.currentChannel.id, content, replyTo });
    await selectChannel(state.currentChannel);
  } catch (e) {
    toast(String(e));
  }
}

function openAddExpression(kind) {
  $("emoji-picker").classList.add("hidden");
  if (kind === "emoticons") openEmoticonModal();
  else openCatalog(kind);
}

const catalog = { kind: "emojis", source: "emoji.gg", query: "", category: null, page: 0, results: [], picked: null, seq: 0, timer: null, busy: false };
const KLIPY_CATALOG_CHIPS = [{ id: null, name: "Stickers" }, { id: "gifs", name: "GIFs" }];

function catalogSource(kind) {
  if (kind === "stickers" && account.server_stickers) return "KLIPY";
  return account.server_emojis ? "emoji.gg" : null;
}

function syncCatalogBrowse() {
  const source = catalogSource("stickers");
  $("sticker-browse").textContent = `Browse ${source || "KLIPY"}`;
  return source;
}
const CATALOG_SIZES = {
  emojis: { size: 128, maxGif: 256 * 1024, command: "add_emoji", max: MAX_EMOJIS, list: "emojis" },
  stickers: { size: 320, maxGif: 512 * 1024, command: "add_sticker", max: MAX_STICKERS, list: "stickers" },
};

function openCatalog(kind, { upload = false } = {}) {
  const server = state.currentServer;
  if (!server) return;
  closeFloating();
  const source = catalogSource(kind);
  Object.assign(catalog, { kind, source, query: "", category: null, page: 0, results: [], picked: null, busy: false });
  $("catalog-title").textContent = kind === "stickers" ? "Add a sticker" : "Add an emoji";
  $("catalog-search").value = "";
  $("catalog-search").placeholder = source === "KLIPY" ? "Search KLIPY stickers and GIFs" : "Search emoji.gg";
  $("catalog-attrib-source").textContent = source || "emoji.gg";
  $("catalog-chips").replaceChildren();
  renderCatalogPick();
  showModal("modal-catalog");
  if (source) {
    $("catalog-search").disabled = false;
    $("catalog-search").focus();
    loadCatalog();
  } else {
    $("catalog-search").disabled = true;
    $("catalog-grid").replaceChildren(el("div", "catalog-note", "Sign in to an account server with image search to browse. You can still upload an image."));
  }
  if (upload) uploadToCatalog();
}

async function loadCatalog(append = false) {
  const seq = ++catalog.seq;
  const grid = $("catalog-grid");
  if (!append) grid.replaceChildren(el("div", "catalog-note", `Loading ${catalog.source}`));
  try {
    const klipy = catalog.source === "KLIPY";
    const page = klipy
      ? (await invoke("sticker_search", { query: catalog.query, from: catalog.category || "stickers", page: catalog.page + 1 })) || { results: [] }
      : await invoke("search_emoji_catalog", { query: catalog.query, category: catalog.category, page: catalog.page });
    if (seq !== catalog.seq) return;
    if (!$("catalog-chips").childElementCount) {
      renderCatalogChips(klipy ? KLIPY_CATALOG_CHIPS : [{ id: null, name: "Popular" }, ...(page.categories || [])]);
    }
    catalog.results = append ? catalog.results.concat(page.results) : page.results;
    renderCatalogGrid(Boolean(page.more));
  } catch (e) {
    if (seq === catalog.seq) grid.replaceChildren(el("div", "catalog-note", String(e)));
  }
}

function renderCatalogChips(categories) {
  const row = $("catalog-chips");
  row.replaceChildren();
  for (const c of categories) {
    const chip = el("button", "catalog-chip" + (c.id === catalog.category ? " active" : ""), c.name);
    chip.type = "button";
    chip.onclick = () => {
      catalog.category = c.id;
      catalog.page = 0;
      for (const other of row.children) other.classList.toggle("active", other === chip);
      loadCatalog();
    };
    row.appendChild(chip);
  }
}

function renderCatalogGrid(more) {
  const grid = $("catalog-grid");
  grid.replaceChildren();
  for (const item of catalog.results) {
    const tile = el("button", "catalog-tile" + (catalog.picked && catalog.picked.id === item.id ? " picked" : ""));
    tile.type = "button";
    tile.title = item.title;
    const img = el("img");
    img.loading = "lazy";
    img.alt = item.title;
    img.draggable = false;
    img.src = viaAccountServer(item.preview || item.url);
    img.onerror = () => tile.classList.add("missing");
    tile.appendChild(img);
    if (item.animated) tile.appendChild(el("span", "catalog-gif", "GIF"));
    tile.onclick = () => {
      catalog.picked = { ...item, src: img.src };
      for (const t of grid.querySelectorAll(".catalog-tile.picked")) t.classList.remove("picked");
      tile.classList.add("picked");
      renderCatalogPick();
    };
    grid.appendChild(tile);
  }
  if (!catalog.results.length) grid.appendChild(el("div", "catalog-note", `Nothing on ${catalog.source} matches that`));
  if (more) {
    const next = el("button", "btn-secondary btn-small catalog-more", "Load More");
    next.onclick = () => {
      catalog.page += 1;
      next.remove();
      loadCatalog(true);
    };
    grid.appendChild(next);
  }
}

function renderCatalogPick() {
  const picked = catalog.picked;
  const preview = $("catalog-pick-preview");
  preview.replaceChildren();
  preview.classList.toggle("sticker", catalog.kind === "stickers");
  if (picked) {
    const img = el("img");
    img.src = picked.src;
    img.alt = picked.title;
    preview.appendChild(img);
  } else {
    preview.appendChild(svgIcon(catalog.kind === "stickers" ? "uploadFile" : "smile", 22));
  }
  const server = state.currentServer;
  const taken = server ? (server[CATALOG_SIZES[catalog.kind].list] || []).map(e => e.name) : [];
  const name = $("catalog-name");
  name.disabled = !picked;
  name.value = picked ? freeExpressionName(expressionName(picked.title, catalog.kind === "stickers" ? "sticker" : "emoji"), taken) : "";
  $("catalog-add").disabled = !picked || catalog.busy;
}

async function uploadToCatalog() {
  let src;
  try {
    src = await chooseImage(catalog.kind === "stickers" ? "Choose a sticker image" : "Choose an emoji image", $("server-image-file"));
  } catch (e) {
    toast(String(e));
    return;
  }
  if (!src) return;
  catalog.picked = { id: null, title: catalog.kind === "stickers" ? "sticker" : "emoji", src, local: true };
  for (const t of $("catalog-grid").querySelectorAll(".catalog-tile.picked")) t.classList.remove("picked");
  renderCatalogPick();
  $("catalog-name").focus();
  $("catalog-name").select();
}

async function addFromCatalog() {
  const server = state.currentServer;
  const picked = catalog.picked;
  if (!server || !picked || catalog.busy) return;
  const spec = CATALOG_SIZES[catalog.kind];
  const name = expressionName($("catalog-name").value, "");
  if (!name) {
    toast("Names are 2 to 32 letters, numbers or underscores");
    return;
  }
  const items = server[spec.list] || [];
  const exists = items.some(e => e.name === name);
  if (exists && !server.can_manage_expressions) {
    toast(`:${name}: is taken, pick another name`);
    return;
  }
  if (exists && !confirm(`Replace the existing :${name}:?`)) return;
  if (!exists && items.length >= spec.max) {
    toast(`This server already has ${spec.max} ${catalog.kind}`);
    return;
  }
  catalog.busy = true;
  $("catalog-add").disabled = true;
  try {
    const src = picked.local ? picked.src : await invoke("fetch_catalog_image", { url: picked.url });
    const prepared = await prepareEmojiImage(src, spec);
    if (prepared.flattened) toast(`That GIF is over ${spec.maxGif / 1024} KB, so it was saved as a still image`);
    await invoke(spec.command, { serverId: server.id, name, data: prepared.data });
    await refreshExpressions();
    catalog.picked = null;
    for (const t of $("catalog-grid").querySelectorAll(".catalog-tile.picked")) t.classList.remove("picked");
    toast(`Added :${name}: to ${server.name}`);
  } catch (e) {
    toast(String(e.message || e));
  } finally {
    catalog.busy = false;
    renderCatalogPick();
  }
}

function openEmoticonModal(text = "", name = "") {
  if (!state.currentServer) return;
  closeFloating();
  const ideas = $("emoticon-ideas");
  ideas.replaceChildren();
  for (const idea of builtinEmoticons().filter((_, i) => i % 3 === 0).slice(0, 12)) {
    const chip = el("button", "emoticon-idea", idea.text);
    chip.type = "button";
    chip.title = idea.name;
    chip.onclick = () => {
      $("emoticon-text").value = idea.text;
      $("emoticon-name").value = idea.name;
    };
    ideas.appendChild(chip);
  }
  $("emoticon-text").value = text;
  $("emoticon-name").value = name;
  showModal("modal-emoticon");
  $("emoticon-text").focus();
}

async function saveEmoticon() {
  const server = state.currentServer;
  if (!server) return;
  const text = $("emoticon-text").value.trim();
  const name = expressionName($("emoticon-name").value, "");
  if (!text) {
    toast("Type an emoticon first");
    return;
  }
  if (!name) {
    toast("Names are 2 to 32 letters, numbers or underscores");
    return;
  }
  const exists = (server.emoticons || []).some(e => e.name === name);
  if (exists && !server.can_manage_expressions) {
    toast(`:${name}: is taken, pick another name`);
    return;
  }
  if (exists && !confirm(`Replace the existing :${name}:?`)) return;
  try {
    await invoke("add_emoticon", { serverId: server.id, name, text });
    hideModals();
    await refreshExpressions();
    toast(`Added :${name}: to ${server.name}`);
  } catch (e) {
    toast(String(e));
  }
}

function insertIntoComposer(text) {
  const input = $("message-input");
  const pos = input.selectionStart ?? input.value.length;
  input.value = input.value.slice(0, pos) + text + input.value.slice(pos);
  input.focus();
  const next = pos + text.length;
  input.setSelectionRange(next, next);
  fitComposer(input);
}

function fitComposer(input) {
  input.rows = composerRows(input.value);
}

function wireComposerFormatting(input) {
  input.addEventListener("input", () => fitComposer(input));
  input.addEventListener("keydown", e => {
    const marker = formatShortcut(e);
    if (!marker) return;
    e.preventDefault();
    const wrapped = toggleWrap(input.value, input.selectionStart, input.selectionEnd, marker);
    input.value = wrapped.value;
    input.setSelectionRange(wrapped.start, wrapped.end);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const PERMISSION_LABELS = [
  ["manage_server", "Manage Server"],
  ["manage_channels", "Manage Channels"],
  ["manage_messages", "Manage Messages"],
  ["pin_messages", "Pin Messages"],
  ["bypass_slow_mode", "Bypass Slow Mode"],
  ["attach_files", "Send Files and Images"],
  ["send_voice_messages", "Send Voice Messages"],
  ["send_tts_messages", "Send Text-to-Speech Messages"],
  ["embed_links", "Embed Links"],
  ["read_message_history", "Read Message History"],
  ["create_threads", "Create Threads"],
  ["send_polls", "Create Polls"],
  ["add_emojis", "Add Emojis"],
  ["add_stickers", "Add Stickers"],
  ["add_emoticons", "Add Emoticons"],
  ["use_external_emojis", "Use Other Servers' Emojis and Stickers"],
  ["post_announcements", "Post Announcements"],
  ["mention_everyone", "Mention Everyone and Any Role"],
  ["manage_expressions", "Manage Expressions"],
  ["create_events", "Create Events"],
  ["manage_events", "Manage Events"],
  ["manage_roles", "Manage Roles"],
  ["kick_members", "Kick Members"],
  ["create_invites", "Create Invites"],
  ["view_audit_log", "View Audit Log"],
  ["ban_members", "Ban Members"],
  ["mute_members", "Mute Members"],
  ["time_out_members", "Time Out Members"],
  ["priority_speaker", "Priority Speaker"],
  ["manage_nicknames", "Manage Nicknames"],
  ["change_nickname", "Change Nickname"],
  ["move_members", "Move Members"],
  ["administrator", "Administrator"],
];

const iconState = { custom: { app: null, tray: null } };

async function renderIconPickers() {
  for (const target of ["app", "tray"]) {
    try {
      iconState.custom[target] = await invoke("get_custom_icon", { target });
    } catch {
      iconState.custom[target] = null;
    }
    const grid = $(`${target}-icon-grid`);
    const current = (target === "app" ? state.voice.app_icon : state.voice.tray_icon) || (target === "app" ? "classic" : "app");
    grid.replaceChildren();
    for (const choice of appIconChoices(target, Boolean(iconState.custom[target]))) {
      const tile = el("button", "icon-tile" + (choice.id === current ? " active" : ""));
      tile.type = "button";
      tile.setAttribute("role", "radio");
      tile.setAttribute("aria-checked", String(choice.id === current));
      const art = el("span", "icon-art");
      if (choice.id === "app") {
        art.appendChild(svgIcon("check", 18));
      } else if (choice.custom && !iconState.custom[target]) {
        art.appendChild(svgIcon("plus", 18));
      } else {
        const img = el("img", choice.custom ? "" : "icon-mark");
        img.alt = "";
        img.draggable = false;
        img.src = choice.custom ? iconState.custom[target] : choice.image;
        if (choice.tile) art.style.background = choice.tile;
        art.appendChild(img);
      }
      tile.append(art, el("span", "icon-label", choice.label));
      tile.onclick = () => (choice.custom && !iconState.custom[target] ? pickCustomIcon(target) : saveIconChoice(target, choice.id));
      grid.appendChild(tile);
    }
    if (iconState.custom[target]) {
      const replace = el("button", "btn-secondary btn-small icon-replace", "Choose another image");
      replace.type = "button";
      replace.onclick = () => pickCustomIcon(target);
      grid.appendChild(replace);
    }
  }
}

async function saveIconChoice(target, id) {
  const next = { app_icon: state.voice.app_icon || "classic", tray_icon: state.voice.tray_icon || "app" };
  next[`${target}_icon`] = id;
  try {
    state.voice = await invoke("set_icons", { appIcon: next.app_icon, trayIcon: next.tray_icon });
  } catch (err) {
    toast(String(err));
  }
  renderIconPickers();
}

async function pickCustomIcon(target) {
  try {
    const settings = await invoke("choose_custom_icon", { target });
    if (settings) state.voice = settings;
  } catch (err) {
    toast(String(err));
  }
  renderIconPickers();
}

function openUserSettings(tab) {
  if (!state.user) return;
  renderIconPickers();
  $("nc-toggle").checked = state.voice.noise_cancellation;
  $("ec-toggle").checked = state.voice.echo_cancellation !== false;
  $("agc-toggle").checked = state.voice.auto_level !== false;
  $("afk-timeout").value = String(state.voice.afk_timeout_minutes ?? 10);
  $("share-activity-toggle").checked = state.voice.share_activity !== false;
  loadAudioDevices();
  loadCameraDevices();
  renderSoundSettings();
  loadSafety().then(renderSafetySettings);
  loadStorageSettings().then(renderStorageSettings);
  $("account-about").value = decodeCustomEmojis(state.user.about);
  $("account-pronouns").value = state.voice.pronouns || "";
  loadOverlaySettings().then(renderOverlaySettings);
  loadUpdateStatus();
  if (!document.documentElement.classList.contains("web-app")) loadDebugLogStatus();
  loadIntegrations().then(renderIntegrations);
  loadStreamDeck().then(renderStreamDeck);
  loadAccount().then(() => {
    renderAccountSettings();
    $("gif-provider").dispatchEvent(new Event("change"));
  });
  $("notify-level").value = notifyPrefs().level;
  $("friend-online-toggle").checked = Boolean(notifyPrefs().friendOnline);
  renderTtsSettings();
  $("close-to-tray-toggle").checked = state.voice.close_to_tray !== false;
  invoke("get_autostart").then(on => { $("autostart-toggle").checked = on; }).catch(() => {});
  invoke("autostart_allowed").then(ok => { $("autostart-toggle").closest("label").classList.toggle("hidden", !ok); }).catch(() => {});
  $("output-vol").value = Math.round((state.voice.output_volume ?? 1) * 100);
  $("input-gain").value = Math.round((state.voice.input_gain ?? 1) * 100);
  $("output-vol-label").textContent = `${$("output-vol").value}%`;
  $("input-gain-label").textContent = `${$("input-gain").value}%`;
  renderVoiceInputSettings();
  startMicMeter();
  $("account-display-name").value = state.user.display_name || "";
  $("account-status").value = decodeCustomEmojis(state.user.custom_status);
  accountStatusEmoji = state.user.status_emoji || "";
  paintEmojiSlot($("account-status-emoji"), accountStatusEmoji);
  $("account-presence").value = state.user.presence || "online";
  $("account-avatar").replaceChildren();
  paintAvatar($("account-avatar"), state.user.id, state.user.display_name || state.user.username);
  paintAccountBanner();
  $("account-preview-name").textContent = state.user.display_name || state.user.username;
  $("account-username").textContent = state.user.username;
  openSettingsLayer("user-settings", tab);
}

function canOpenServerSettings(server) {
  const isOwner = Boolean(state.user && server.owner === state.user.id);
  return isOwner || Boolean(server.can_manage_server || server.can_manage_channels || server.can_manage_roles ||
    server.can_kick || server.can_manage_expressions || server.can_upload_sounds);
}

function openServerInfo(server) {
  const me = state.user && server.members.find(m => m.id === state.user.id);
  const online = server.members.filter(m => m.presence && m.presence !== "offline").length;
  paintServerIcon($("si-icon"), server);
  $("si-title").textContent = server.name;
  const count = server.members.length;
  $("si-sub").textContent = [`${count} member${count === 1 ? "" : "s"}`, online ? `${online} online` : "", server.public ? "Public server" : ""].filter(Boolean).join(" \u00b7 ");
  const banner = $("si-banner");
  banner.style.backgroundImage = "";
  banner.classList.toggle("hidden", !server.banner);
  if (server.banner) {
    const ref = server.banner;
    loadServerImage(ref).then(url => { if (url && server.banner === ref) banner.style.backgroundImage = `url("${url}")`; });
  }
  const games = $("si-games");
  games.replaceChildren();
  for (const tag of server.game_tags || []) {
    games.appendChild(gameChip(server, tag, "game-chip small"));
  }
  games.classList.toggle("hidden", !games.childElementCount);
  $("si-rules-wrap").classList.toggle("hidden", !server.rules);
  $("si-rules").replaceChildren(server.rules ? renderMarkdown(server.rules) : "");
  const held = (server.roles || []).filter(r => me && (me.roles || []).includes(r.id));
  const roles = $("si-roles");
  roles.replaceChildren();
  for (const role of held) {
    const chip = el("span", "si-role");
    const dot = el("span", "role-dot");
    if (role.color) dot.style.background = `#${role.color}`;
    chip.append(dot, document.createTextNode(role.name));
    if (role.tag) chip.appendChild(roleTagChip(role.tag, role.color));
    roles.appendChild(chip);
  }
  $("si-roles-wrap").classList.toggle("hidden", !held.length);
  $("si-answers").classList.toggle("hidden", !server.onboarding);
  showModal("modal-server-info");
}

const TEMPLATE_TILE_ICONS = { fresh: "plus", friends: "people", gaming: "gamepad", community: "megaphone", study: "book", code: "paste" };
const templatePicker = { list: [], choice: "fresh", code: null, preview: null };

async function openCreateServer(code = "") {
  try {
    if (!templatePicker.list.length) templatePicker.list = await invoke("list_server_templates");
  } catch (e) {
    return toast(String(e));
  }
  Object.assign(templatePicker, { choice: code ? "code" : "fresh", code: null, preview: null });
  $("template-code-input").value = code;
  $("create-name-input").value = "";
  renderTemplateGrid();
  renderTemplatePreview();
  showModal("modal-create-server");
  if (code) await readTemplateCode();
  else $("create-name-input").focus();
}

function renderTemplateGrid() {
  const tiles = [
    ...templatePicker.list.map(t => ({ id: t.id, title: t.title, blurb: t.blurb })),
    { id: "code", title: "Use a template code", blurb: "Copy another server's layout" },
  ];
  $("template-grid").replaceChildren(...tiles.map(tile => {
    const button = document.createElement("button");
    button.className = "template-tile";
    button.setAttribute("role", "radio");
    button.dataset.template = tile.id;
    const icon = document.createElement("span");
    icon.className = "template-tile-icon";
    icon.appendChild(svgIcon(TEMPLATE_TILE_ICONS[tile.id] || "plus", 20));
    const title = document.createElement("span");
    title.className = "template-tile-title";
    title.textContent = tile.title;
    const blurb = document.createElement("span");
    blurb.className = "template-tile-blurb";
    blurb.textContent = tile.blurb;
    button.append(icon, title, blurb);
    button.onclick = () => selectTemplate(tile.id);
    return button;
  }));
  markChosenTile();
}

function markChosenTile() {
  for (const tile of $("template-grid").children) {
    tile.setAttribute("aria-checked", String(tile.dataset.template === templatePicker.choice));
  }
}

function selectTemplate(id) {
  templatePicker.choice = id;
  markChosenTile();
  renderTemplatePreview();
  if (id === "code" && !templatePicker.preview) $("template-code-input").focus();
}

function chosenTemplate() {
  return templatePicker.choice === "code" ? templatePicker.code : templatePicker.choice;
}

function templateChip(name, kind) {
  const chip = document.createElement("span");
  chip.className = "template-chip";
  if (isVoiceKind(kind)) chip.appendChild(voiceKindIcon(kind, 14));
  else if (kind === "announcement") chip.appendChild(svgIcon("megaphone", 14));
  else {
    const hash = document.createElement("span");
    hash.className = "template-hash";
    hash.textContent = "#";
    chip.appendChild(hash);
  }
  chip.append(name);
  return chip;
}

function templateRoleChip(role) {
  const chip = document.createElement("span");
  chip.className = "template-chip";
  const dot = document.createElement("span");
  dot.className = "template-role-dot";
  if (role.color) dot.style.background = `#${role.color}`;
  chip.append(dot, role.name);
  return chip;
}

function templateGroupNode(label, chips) {
  const group = document.createElement("div");
  group.className = "template-group";
  const name = document.createElement("div");
  name.className = "template-group-name";
  name.textContent = label;
  name.title = name.textContent;
  const list = document.createElement("div");
  list.className = "template-group-chips";
  list.append(...chips);
  group.append(name, list);
  return group;
}

function renderTemplatePreview() {
  const fromCode = templatePicker.choice === "code";
  const template = fromCode ? templatePicker.preview : templatePicker.list.find(t => t.id === templatePicker.choice);
  $("template-code-entry").classList.toggle("hidden", Boolean(!fromCode || template));
  $("template-preview-view").classList.toggle("hidden", !template);
  $("template-code-change").classList.toggle("hidden", !fromCode);
  $("create-server-save").disabled = !template;
  if (!template) return;
  $("template-preview-summary").textContent = templateSummary(template);
  const groups = templateGroups(template).map(g => templateGroupNode(g.category, g.channels.map(c => templateChip(c.name, c.kind))));
  if (template.roles.length) groups.push(templateGroupNode("Roles", template.roles.map(templateRoleChip)));
  $("template-preview-body").replaceChildren(...groups);
  $("template-preview-body").scrollTop = 0;
}

async function readTemplateCode() {
  const input = $("template-code-input");
  const note = $("template-code-note");
  const code = input.value.trim();
  note.classList.remove("error");
  note.textContent = "Ask a server admin to copy it from Server Settings, then Overview.";
  templatePicker.preview = null;
  templatePicker.code = null;
  if (code && !looksLikeTemplateCode(code)) {
    note.classList.add("error");
    note.textContent = "Template codes start with commy-template:";
  } else if (code) {
    try {
      const preview = await invoke("preview_server_template", { code });
      if (input.value.trim() !== code) return;
      Object.assign(templatePicker, { preview, code });
      if (!$("create-name-input").value.trim()) $("create-name-input").value = preview.name;
    } catch (e) {
      note.classList.add("error");
      note.textContent = String(e);
    }
  }
  renderTemplatePreview();
}

function openServerSettings(tab) {
  if (!state.currentServer) return;
  const server = state.currentServer;
  if (!canOpenServerSettings(server)) return openServerInfo(server);
  const layer = $("server-settings");
  const canMembers = server.can_kick || server.can_manage_roles;
  $("server-settings-title").textContent = server.name;

  $("rename-input").value = server.name;
  $("rename-input").disabled = !server.can_manage_server;
  $("rename-save").classList.toggle("hidden", !server.can_manage_server);
  $("overview-readonly").classList.toggle("hidden", server.can_manage_server);
  $("server-public-toggle").checked = Boolean(server.public);
  $("server-public-toggle").disabled = !server.can_manage_server;
  $("server-public-row").classList.toggle("readonly", !server.can_manage_server);
  $("server-activities-toggle").checked = Boolean(server.activities_enabled);
  $("server-activities-toggle").disabled = !server.can_manage_server;
  $("server-activities-row").classList.toggle("readonly", !server.can_manage_server);
  discoveryState.dirty = false;
  paintDiscoverySettings(server);
  $("server-template-section").classList.toggle("hidden", !server.can_manage_server);
  const afkSelect = $("afk-channel-select");
  fillVoiceChannelSelect(afkSelect, "No AFK channel", server, server.afk_channel || "");
  afkSelect.disabled = !server.can_manage_server;
  $("server-afk-timeout").value = String(server.afk_timeout_minutes || 0);
  $("server-afk-timeout").disabled = !server.can_manage_server;
  paintServerImages(server);
  $("tab-channels").classList.toggle("hidden", !server.can_manage_channels);
  $("tab-roles").classList.toggle("hidden", !server.can_manage_roles);
  $("tab-members").classList.toggle("hidden", !canMembers);
  const manages = server.can_manage_expressions || server.can_manage_server;
  const canEmoji = manages || server.can_add_emojis;
  const canStickers = manages || server.can_add_stickers;
  const canEmoticons = manages || server.can_add_emoticons;
  const canSounds = manages || server.can_upload_sounds;
  $("tab-rules").classList.toggle("hidden", !server.can_manage_server);
  renderRulesSettings(server);
  $("tab-onboarding").classList.toggle("hidden", !server.can_manage_server);
  if (server.can_manage_server) renderOnboardingSettings(server);
  $("tab-automod").classList.toggle("hidden", !server.can_manage_server);
  if (server.can_manage_server) renderAutomodSettings(server);
  renderGameTagEditor(server);
  const isOwner = Boolean(state.user && server.owner === state.user.id);
  $("tab-reports").classList.toggle("hidden", !isOwner);
  if (isOwner) renderReportSettings(server);
  $("server-delete-section").classList.toggle("hidden", !isOwner);
  $("tab-audit").classList.toggle("hidden", !server.can_view_audit_log);
  if (server.can_view_audit_log) renderAuditLog(server);
  $("tab-emoji").classList.toggle("hidden", !canEmoji);
  $("tab-stickers").classList.toggle("hidden", !canStickers);
  $("tab-emoticons").classList.toggle("hidden", !canEmoticons);
  $("tab-soundboard").classList.toggle("hidden", !canSounds);
  const canInvites = Boolean(server.can_invite || server.can_manage_server);
  $("tab-invites").classList.toggle("hidden", !canInvites);
  if (canInvites) renderInvitesPage(server);
  const canBots = canSeeBotsPage(server);
  $("tab-bots").classList.toggle("hidden", !canBots);
  if (canBots) renderBotsPage(server);
  const allowed = {
    bots: canBots,
    invites: canInvites,
    overview: true,
    discovery: true,
    channels: server.can_manage_channels,
    roles: server.can_manage_roles,
    emoji: canEmoji,
    stickers: canStickers,
    emoticons: canEmoticons,
    soundboard: canSounds,
    reports: isOwner,
    rules: server.can_manage_server,
    onboarding: server.can_manage_server,
    automod: server.can_manage_server,
    members: canMembers,
    audit: server.can_view_audit_log,
  };
  renderEmojiSettings(server);
  renderStickerSettings(server);
  renderEmoticonSettings(server);
  renderSoundboardSettings(server);
  hideEmptyNavGroups(layer);
  let current = tab || (layer.classList.contains("hidden") ? "overview" : layer.dataset.tab);
  if (!allowed[current]) current = "overview";

  const list = $("manage-channel-list");
  list.innerHTML = "";
  for (const channel of server.channels) {
    const row = el("div", "manage-row");
    const label = el("span", "manage-row-label");
    const icon = el("span", "channel-icon");
    if (isVoiceKind(channel.kind)) icon.appendChild(voiceKindIcon(channel.kind, 18));
    else icon.textContent = "#";
    label.appendChild(icon);
    label.appendChild(channelNameNode(channel));
    label.appendChild(el("span", "kind", channel.kind));
    row.appendChild(label);
    const edit = el("button", "btn-secondary btn-small", "Edit");
    edit.onclick = () => openChannelSettings(channel.id);
    row.appendChild(edit);
    list.appendChild(row);
  }

  if (!server.channels.length) list.appendChild(el("div", "manage-empty", "No channels yet"));

  renderRoleSettings(server);
  renderMemberSettings(server);
  openSettingsLayer("server-settings", current);
}

const auditView = { serverId: null, entries: [] };

async function renderAuditLog(server) {
  try {
    const entries = await invoke("audit_log", { serverId: server.id });
    if (!state.currentServer || state.currentServer.id !== server.id) return;
    const fresh = auditView.serverId !== server.id;
    Object.assign(auditView, { serverId: server.id, entries });
    fillAuditFilters(fresh);
    paintAuditLog();
  } catch (e) {
    auditView.entries = [];
    $("audit-list").replaceChildren(el("div", "manage-empty", String(e)));
  }
}

function fillAuditFilters(reset) {
  const group = $("audit-group");
  const actor = $("audit-actor");
  const keepGroup = reset ? "all" : group.value || "all";
  const keepActor = reset ? "" : actor.value;
  group.replaceChildren(...AUDIT_FILTERS.map(([id, label]) => new Option(label, id)));
  group.value = keepGroup;
  const actors = auditActors(auditView.entries);
  actor.replaceChildren(new Option("Everyone", ""), ...actors.map(a => new Option(a.name, a.id)));
  actor.value = actors.some(a => a.id === keepActor) ? keepActor : "";
}

function auditRow(entry) {
  const row = el("div", "audit-row");
  row.appendChild(makeAvatar(entry.by, entry.by_name, "avatar avatar-sm"));
  const text = el("div", "audit-text");
  for (const part of auditParts(entry)) text.appendChild(part.strong ? el("strong", null, part.text) : document.createTextNode(part.text));
  text.title = auditText(entry);
  row.appendChild(text);
  const when = new Date(entry.at);
  const time = el("time", "audit-time", formatTime(when.toISOString()));
  time.dateTime = when.toISOString();
  time.title = when.toLocaleString();
  row.appendChild(time);
  return row;
}

function paintAuditLog() {
  const list = $("audit-list");
  const shown = filterAudit(auditView.entries, { group: $("audit-group").value || "all", by: $("audit-actor").value });
  if (!shown.length) {
    const empty = auditView.entries.length ? "Nothing matches these filters." : "No changes yet. Settings, channel, role and member changes show up here.";
    list.replaceChildren(el("div", "manage-empty", empty));
    return;
  }
  list.replaceChildren(...shown.map(auditRow));
}

const INVITE_BACKGROUND_MAX = 400 * 1024;
const invitesView = { serverId: null, invites: [], clock: null, accountHost: null, accountOrigin: null };

async function renderInvitesPage(server) {
  const manage = Boolean(server.can_manage_server);
  $("invite-manage").classList.toggle("hidden", !manage);
  if (manage) {
    paintInviteBackground(server);
    const never = $("invite-custom-never");
    never.disabled = !server.public;
    never.textContent = server.public ? "Never expires" : "Never expires (public servers only)";
    if (!server.public && $("invite-custom-expiry").value === "never") $("invite-custom-expiry").value = "30d";
    if (invitesView.serverId !== server.id) $("invite-custom-name").value = "";
    paintInviteNameCheck();
    if (!invitesView.accountHost) {
      invoke("account_server_info").then(info => {
        invitesView.accountHost = info.host;
        invitesView.accountOrigin = info.origin;
        $("invite-custom-prefix").textContent = `${info.origin}/i/`;
      }).catch(() => {});
    } else {
      $("invite-custom-prefix").textContent = `${invitesView.accountOrigin}/i/`;
    }
  }
  try {
    const invites = await invoke("list_invites", { serverId: server.id });
    if (!state.currentServer || state.currentServer.id !== server.id) return;
    Object.assign(invitesView, { serverId: server.id, invites: sortInvites(invites) });
  } catch (e) {
    invitesView.invites = [];
    $("invite-list").replaceChildren(el("div", "manage-empty", String(e)));
    return;
  }
  paintInviteList();
  startInviteClock();
}

function paintInviteBackground(server) {
  const preview = $("invite-bg-preview");
  const ref = server.invite_background || null;
  preview.style.backgroundImage = "";
  preview.classList.toggle("empty", !ref);
  $("invite-bg-remove").classList.toggle("hidden", !ref);
  if (ref) {
    loadServerImage(ref).then(url => {
      if (url && state.currentServer?.invite_background === ref) preview.style.backgroundImage = `url("${url}")`;
    });
  }
}

async function uploadInviteBackground() {
  try {
    const src = await chooseImage("Choose an invite background", $("server-image-file"));
    if (!src) return;
    let data = null;
    for (const quality of [0.86, 0.76, 0.66, 0.56, 0.46]) {
      data = await cropImageFile(src, 1280, 720, quality);
      if (data.length * 3 / 4 <= INVITE_BACKGROUND_MAX) break;
      data = null;
    }
    if (!data) throw "That image is too detailed to fit. Try a simpler one.";
    await setServerImage("invite", data);
  } catch (err) {
    toast(String(err));
  }
}

function paintInviteNameCheck() {
  const { name, error } = checkInviteName($("invite-custom-name").value);
  const note = $("invite-custom-note");
  note.classList.toggle("error", Boolean(error));
  note.textContent = error || "Pick a name people can remember. You need to be signed in. The link ends in a short key after #, so always share the whole link. That key keeps your account server from learning which server it opens.";
  $("invite-custom-create").disabled = !name || Boolean(error);
}

async function createCustomInvite() {
  const server = state.currentServer;
  const { name, error } = checkInviteName($("invite-custom-name").value);
  if (!server || !name || error) return;
  const button = $("invite-custom-create");
  button.disabled = true;
  try {
    const invite = await invoke("create_invite_link", { serverId: server.id, expires: $("invite-custom-expiry").value, name });
    await copyFullInviteLink(invite.url);
    $("invite-custom-name").value = "";
    await renderInvitesPage(server);
  } catch (e) {
    toast(String(e));
  } finally {
    paintInviteNameCheck();
  }
}

function inviteRow(invite) {
  const row = el("div", "invite-row");
  row.appendChild(makeAvatar(invite.creator, invite.creator_name, "avatar avatar-sm"));
  const text = el("div", "invite-row-text");
  const label = el("div", "invite-row-link", inviteLabel(invite));
  label.title = inviteLabel(invite);
  text.appendChild(label);
  const made = new Date(invite.created_at);
  text.appendChild(el("div", "invite-row-meta", `${invite.custom ? "Custom link" : "Invite"} by ${invite.creator_name} on ${made.toLocaleDateString(undefined, { month: "short", day: "numeric" })}`));
  row.appendChild(text);
  const left = el("span", "invite-row-left", inviteTimeLeft(invite.expires_at));
  left.dataset.expires = invite.expires_at == null ? "" : String(invite.expires_at);
  if (invite.expires_at != null) left.title = `Expires ${new Date(invite.expires_at).toLocaleString()}`;
  row.appendChild(left);
  const copy = el("button", "btn-secondary btn-small invite-row-copy", "Copy Link");
  copy.onclick = () => copyTrackedInvite(invite, copy);
  row.appendChild(copy);
  const revoke = el("button", "btn-secondary btn-small danger-text invite-row-revoke", "Revoke");
  revoke.disabled = !invite.can_revoke;
  revoke.onclick = () => revokeInvite(invite, revoke);
  row.appendChild(revoke);
  return row;
}

function paintInviteList() {
  const shown = invitesView.invites.filter(i => i.expires_at == null || i.expires_at > Date.now());
  if (!shown.length) {
    $("invite-list").replaceChildren(el("div", "manage-empty", "No active invites. Links made with Invite People show up here until they expire."));
    return;
  }
  $("invite-list").replaceChildren(...shown.map(inviteRow));
}

function startInviteClock() {
  if (invitesView.clock) return;
  invitesView.clock = setInterval(() => {
    if ($("server-settings").classList.contains("hidden")) {
      clearInterval(invitesView.clock);
      invitesView.clock = null;
      return;
    }
    let expired = false;
    for (const node of $("invite-list").querySelectorAll(".invite-row-left")) {
      const at = node.dataset.expires ? Number(node.dataset.expires) : null;
      node.textContent = inviteTimeLeft(at);
      if (at != null && at <= Date.now()) expired = true;
    }
    if (expired) paintInviteList();
  }, 15e3);
}

async function copyFullInviteLink(url) {
  try {
    await navigator.clipboard.writeText(url);
    toast("Invite link copied. Share all of it, including the part after #.");
  } catch {
    toast(`Copy this whole link: ${url}`);
  }
}

async function copyTrackedInvite(invite, button) {
  const server = state.currentServer;
  if (!server) return;
  button.disabled = true;
  try {
    await copyFullInviteLink(await invoke("invite_full_link", { serverId: server.id, origin: invite.origin, code: invite.code }));
  } catch (e) {
    toast(String(e));
  } finally {
    button.disabled = false;
  }
}

async function revokeInvite(invite, button) {
  const server = state.currentServer;
  if (!server) return;
  button.disabled = true;
  try {
    await invoke("revoke_invite", { serverId: server.id, origin: invite.origin, code: invite.code });
    toast("Invite revoked");
    await renderInvitesPage(server);
  } catch (e) {
    toast(String(e));
    button.disabled = false;
  }
}

const ROLE_ACCESS = {
  files: {
    box: "upload-access", permission: "attach_files", restricted: server => Boolean(server.uploads_restricted),
    title: "Who can send files and images",
    sub: "Applies to attachments, pasted images and dropped files in every channel. Text, links and GIFs are not affected. The owner and anyone with Administrator can always send them.",
  },
  emojis: {
    box: "emoji-access", permission: "add_emojis", restricted: server => Boolean(server.add_restricted?.emojis),
    title: "Who can add emojis",
    sub: "Everyone can use this server's emojis. This decides who can add new ones from emoji.gg or their computer. Replacing and deleting stays with roles that have Manage Expressions.",
  },
  stickers: {
    box: "sticker-access", permission: "add_stickers", restricted: server => Boolean(server.add_restricted?.stickers),
    title: "Who can add stickers",
    sub: "Everyone can send this server's stickers. This decides who can add new ones. Replacing and deleting stays with roles that have Manage Expressions.",
  },
  emoticons: {
    box: "emoticon-access", permission: "add_emoticons", restricted: server => Boolean(server.add_restricted?.emoticons),
    title: "Who can add emoticons",
    sub: "Everyone can use this server's emoticons. This decides who can save new ones. Replacing and deleting stays with roles that have Manage Expressions.",
  },
};

function renderRoleAccess(server, access) {
  const spec = ROLE_ACCESS[access];
  const box = $(spec.box);
  box.replaceChildren();
  const restricted = spec.restricted(server);
  const save = async (nextRestricted, ids) => {
    try {
      await invoke("set_role_access", { serverId: server.id, access, restricted: nextRestricted, roleIds: ids });
      await refreshCurrentServer();
      openServerSettings();
    } catch (err) {
      toast(String(err));
      openServerSettings();
    }
  };
  const allowedIds = () => server.roles.filter(r => (r.permissions || []).includes(spec.permission)).map(r => r.id);
  box.appendChild(el("div", "setting-title", spec.title));
  box.appendChild(el("div", "setting-sub", spec.sub));
  const seg = el("div", "segmented upload-seg");
  for (const [label, hint, value] of [["Anyone", "Every member", false], ["Only selected roles", "Pick the roles below", true]]) {
    const b = el("button", value === restricted ? "active" : "");
    b.type = "button";
    b.disabled = !server.can_manage_roles;
    b.append(el("strong", null, label), el("span", null, hint));
    b.onclick = () => {
      if (value !== restricted) save(value, allowedIds());
    };
    seg.appendChild(b);
  }
  box.appendChild(seg);
  const picks = el("div", "upload-roles" + (restricted ? "" : " off"));
  if (!server.roles.length) picks.appendChild(el("div", "setting-sub", "Create a role first, then pick it here."));
  for (const role of server.roles) {
    const row = el("label", "role-self");
    const check = el("input");
    check.type = "checkbox";
    check.checked = restricted && (role.permissions || []).includes(spec.permission);
    check.disabled = !restricted || !server.can_manage_roles;
    check.onchange = () => {
      const ids = new Set(allowedIds());
      if (check.checked) ids.add(role.id);
      else ids.delete(role.id);
      save(true, [...ids]);
    };
    row.append(check, el("span", null, role.name));
    picks.appendChild(row);
  }
  box.appendChild(picks);
}

function canAttachHere() {
  return !state.currentServer || state.currentServer.can_attach_files !== false;
}

function canSendVoiceHere() {
  return !state.currentServer || state.currentServer.can_send_voice_messages !== false;
}

function syncAttachAccess() {
  const locked = state.view === "server" && !canAttachHere() && !canSendVoiceHere();
  $("attach-btn").classList.toggle("locked", locked);
  $("attach-btn").title = locked ? "You can't send files or voice messages in this server" : "Attach";
}

function renderEveryoneChatAccess(server) {
  const box = $("chat-access");
  box.replaceChildren();
  box.appendChild(el("div", "setting-title", "Everyone can"));
  box.appendChild(el("div", "setting-sub", "What every member may do. Switching one off applies to every role; turn it back on in a role's form to let that role through. The owner and administrators are never held back."));
  const save = async (access, restricted, ids) => {
    try {
      await invoke("set_role_access", { serverId: server.id, access, restricted, roleIds: ids });
      await refreshCurrentServer();
      openServerSettings();
    } catch (err) {
      toast(String(err));
      openServerSettings();
    }
  };
  for (const spec of EVERYONE_CHAT_ACCESS) {
    const row = el("label", "setting-row chat-access-row");
    const text = el("div");
    text.append(el("div", "setting-title", spec.title), el("div", "setting-sub", spec.sub));
    const check = el("input", "switch");
    check.type = "checkbox";
    check.checked = !everyoneLacks(server, spec);
    check.disabled = !server.can_manage_roles;
    check.onchange = () => save(spec.key, !check.checked, []);
    row.append(text, check);
    box.appendChild(row);
  }
}

function renderRoleSettings(server) {
  renderRoleAccess(server, "files");
  renderEveryoneChatAccess(server);
  const list = $("manage-role-list");
  list.innerHTML = "";
  for (const role of server.roles) {
    const card = el("div", "role-card");
    const strip = el("div", "role-card-banner");
    paintBanner(strip, role.banner, role.color ? `#${role.color}` : "var(--bg-active)");
    const tools = el("div", "role-banner-tools");
    const upload = el("button", "role-banner-btn");
    upload.type = "button";
    upload.appendChild(svgIcon("plus", 14));
    upload.appendChild(el("span", null, role.banner ? "Change banner" : "Add banner"));
    upload.onclick = () => uploadRoleBanner(server, role);
    if (role.banner) {
      const remove = el("button", "role-banner-btn icon", "");
      remove.type = "button";
      remove.title = "Remove banner";
      remove.appendChild(svgIcon("close", 14));
      remove.onclick = () => removeRoleBanner(server, role);
      tools.appendChild(remove);
    }
    tools.appendChild(upload);
    strip.appendChild(tools);
    card.appendChild(strip);
    const row = el("div", "role-card-row");
    const label = el("span", "manage-row-label");
    const dot = el("span", "role-dot");
    if (role.color) dot.style.background = `#${role.color}`;
    label.appendChild(dot);
    const name = el("span", "role-card-name", role.name);
    paintRole(name, role.color);
    label.appendChild(name);
    if (role.tag) label.appendChild(roleTagChip(role.tag, role.color));
    if (role.self_assignable) label.appendChild(el("span", "kind pickable", "pickable"));
    if (role.mentionable) label.appendChild(el("span", "kind", "anyone can mention"));
    if (role.hoist) label.appendChild(el("span", "kind", "own group"));
    const granted = grantedNames(role.permissions);
    const count = granted.length;
    const perms = el("span", "kind", count ? (count === 1 ? "1 permission" : `${count} permissions`) : "no permissions");
    if (count) perms.title = granted.map(p => (PERMISSION_LABELS.find(([v]) => v === p) || [p, p])[1]).join(", ");
    label.appendChild(perms);
    row.appendChild(label);
    const edit = el("button", "btn-secondary btn-small", "Edit");
    edit.onclick = () => {
      const open = card.querySelector(".role-form");
      if (open) open.remove();
      else card.appendChild(roleForm(server, role));
    };
    row.appendChild(edit);
    card.appendChild(row);
    list.appendChild(card);
  }
  if (!server.roles.length) list.appendChild(el("div", "manage-empty", "No roles yet"));
}

function renderMemberSettings(server) {
  const list = $("manage-member-list");
  list.innerHTML = "";
  if ((server.banned || []).length) {
    list.appendChild(el("div", "settings-subhead", `Bans - ${server.banned.length}`));
    for (const [id, name] of server.banned) {
      const row = el("div", "manage-row");
      const who = el("span", "manage-row-label");
      who.appendChild(makeAvatar(id, name, "avatar avatar-sm"));
      who.appendChild(el("span", null, name));
      row.appendChild(who);
      const unban = el("button", "btn-secondary btn-small", "Unban");
      unban.onclick = async () => {
        try {
          await invoke("unban_member", { serverId: server.id, userId: id });
          await refreshCurrentServer();
          openServerSettings();
        } catch (e) {
          toast(String(e));
        }
      };
      row.appendChild(unban);
      list.appendChild(row);
    }
    list.appendChild(el("div", "settings-subhead", "Members"));
  }
  for (const member of server.members) {
    const row = el("div", "manage-row manage-member-row");
    const top = el("div", "row-top");
    const who = el("span", "manage-row-label");
    who.appendChild(makeAvatar(member.id, member.name, "avatar avatar-sm"));
    const name = el("span", null, member.name);
    paintRole(name, member.color);
    who.appendChild(name);
    if (member.id === server.owner) who.appendChild(el("span", "kind", "owner"));
    top.appendChild(who);
    const isSelf = state.user && member.id === state.user.id;
    const isOwner = member.id === server.owner;
    if (timeoutActive(member.timeout_until)) who.appendChild(timeoutBadge(member.timeout_until));
    const moderable = canTimeOut(server, member, state.user && state.user.id) || (!isSelf && !isOwner && (server.can_kick || server.can_ban));
    if (moderable) {
      const more = el("button", "icon-btn row-more");
      more.title = `Actions for ${member.name}`;
      more.setAttribute("aria-label", more.title);
      more.setAttribute("aria-haspopup", "menu");
      more.appendChild(svgIcon("more", 18));
      more.onclick = e => {
        e.stopPropagation();
        openMemberRowMenu(member, more);
      };
      top.appendChild(more);
    }
    row.appendChild(top);

    if (server.can_manage_roles && server.roles.length) {
      const chips = el("div", "role-chips");
      for (const role of server.roles) {
        const held = (member.roles || []).includes(role.id);
        const chip = el("span", "role-chip" + (held ? " held" : ""), role.name);
        if (role.color) chip.style.borderColor = `#${role.color}`;
        chip.title = held ? "Click to remove role" : "Click to grant role";
        chip.onclick = async () => {
          try {
            await invoke("set_member_role", {
              serverId: server.id,
              userId: member.id,
              roleId: role.id,
              grant: !held,
            });
            await refreshCurrentServer();
            openServerSettings();
          } catch (e) {
            toast(String(e));
          }
        };
        chips.appendChild(chip);
      }
      row.appendChild(chips);
    }
    list.appendChild(row);
  }
}

function renderBotsPage(server) {
  const bots = serverBots(server);
  const add = $("bots-add");
  add.disabled = !server.can_invite;
  add.title = server.can_invite ? "" : "You need permission to create invites to add a bot";
  $("bots-count").textContent = bots.length ? `Installed - ${bots.length}` : "Installed";
  const list = $("bots-list");
  if (!bots.length) {
    const empty = el("div", "bots-empty");
    empty.appendChild(el("div", "bots-empty-title", "No bots yet"));
    empty.appendChild(el("div", "setting-sub", "Bots can tidy up chat, run slash commands and post updates from your other tools. Add one to see it here."));
    list.replaceChildren(empty);
    return;
  }
  list.replaceChildren(...bots.map(bot => botCard(server, bot)));
}

function botCard(server, bot) {
  const { member } = bot;
  const card = el("div", "bot-card");
  const top = el("div", "bot-card-top");
  const avatar = makeAvatar(member.id, member.name, "avatar bot-card-avatar");
  avatar.appendChild(el("span", "presence-dot " + (bot.online ? member.presence : "offline")));
  top.appendChild(avatar);
  const who = el("div", "bot-card-who");
  const name = el("div", "bot-card-name");
  name.appendChild(el("span", "bot-card-label", member.name));
  name.appendChild(botTag());
  who.appendChild(name);
  who.appendChild(el("div", "bot-card-status" + (bot.online ? " online" : ""), bot.online ? "Online" : "Offline"));
  top.appendChild(who);
  const actions = el("div", "bot-card-actions");
  const removable = member.id !== server.owner && !(state.user && member.id === state.user.id);
  if (server.can_kick && removable) {
    const remove = el("button", "btn-secondary btn-small", "Remove");
    remove.onclick = () => removeBot(server, member, "kick_member");
    actions.appendChild(remove);
  }
  if (server.can_ban && removable) {
    const ban = el("button", "btn-danger btn-small", "Ban");
    ban.onclick = () => removeBot(server, member, "ban_member");
    actions.appendChild(ban);
  }
  top.appendChild(actions);
  card.appendChild(top);
  card.appendChild(el("div", "bot-card-ability", bot.abilities));

  const roles = el("div", "bot-card-row");
  roles.appendChild(el("div", "bot-card-key", "Roles"));
  roles.appendChild(botRoleChips(server, member, async () => {
    await refreshCurrentServer();
    openServerSettings();
  }));
  card.appendChild(roles);

  const commands = el("div", "bot-card-row");
  commands.appendChild(el("div", "bot-card-key", "Commands"));
  const chips = el("div", "bot-card-commands");
  for (const command of bot.commands) {
    const chip = el("span", "bot-command", `/${command.name}`);
    if (command.description) chip.title = command.description;
    chips.appendChild(chip);
  }
  if (!bot.commands.length) chips.appendChild(el("span", "bot-card-none", "No slash commands"));
  commands.appendChild(chips);
  card.appendChild(commands);
  return card;
}

function botRoleChips(server, member, changed) {
  const chips = el("div", "role-chips bot-role-chips");
  const held = new Set(member.roles || []);
  if (!server.can_manage_roles) {
    for (const role of server.roles.filter(r => held.has(r.id))) {
      const chip = el("span", "role-chip held readonly", role.name);
      if (role.color) chip.style.borderColor = `#${role.color}`;
      chips.appendChild(chip);
    }
    if (!chips.children.length) chips.appendChild(el("span", "bot-card-none", "No roles"));
    return chips;
  }
  for (const role of server.roles) {
    const has = held.has(role.id);
    const chip = el("button", "role-chip" + (has ? " held" : ""), role.name);
    if (role.color) chip.style.borderColor = `#${role.color}`;
    chip.title = has ? "Click to remove role" : "Click to grant role";
    chip.onclick = async () => {
      try {
        await invoke("set_member_role", { serverId: server.id, userId: member.id, roleId: role.id, grant: !has });
        await changed();
      } catch (e) {
        toast(String(e));
      }
    };
    chips.appendChild(chip);
  }
  if (!server.roles.length) chips.appendChild(el("span", "bot-card-none", "No roles yet. Make one in Roles."));
  return chips;
}

async function removeBot(server, member, command) {
  const banning = command === "ban_member";
  const question = banning
    ? `Ban ${member.name} from ${server.name}? It is removed and cannot rejoin until unbanned.`
    : `Remove ${member.name} from ${server.name}?`;
  if (!confirm(question)) return;
  try {
    await invoke(command, { serverId: server.id, userId: member.id });
    await refreshCurrentServer();
    openServerSettings();
    toast(banning ? `Banned ${member.name}` : `Removed ${member.name}`);
  } catch (e) {
    toast(String(e));
  }
}

const addBot = { serverId: null, known: [], invite: "", joined: null, timer: null, changed: false };

function botCodeRow(text, ready = true) {
  const row = el("div", "bot-code-row");
  const code = el("code", "bot-code", text);
  code.title = text;
  row.appendChild(code);
  const copy = el("button", "btn-secondary btn-small bot-copy", "Copy");
  copy.disabled = !ready;
  copy.onclick = () => copyText(text, "Copied");
  row.appendChild(copy);
  return row;
}

function paintAddBotCommands() {
  const commands = botSetupCommands($("add-bot-name").value, addBot.invite);
  const ready = Boolean(addBot.invite);
  $("add-bot-commands").replaceChildren(botCodeRow(commands.create), botCodeRow(commands.join, ready), botCodeRow(commands.run));
  $("add-bot-bridge").replaceChildren(botCodeRow(commands.bridge));
}

function paintAddBotInvite(text, ready) {
  $("add-bot-invite").textContent = text;
  $("add-bot-invite").title = ready ? text : "";
  $("add-bot-invite").classList.toggle("muted", !ready);
  $("add-bot-invite-copy").disabled = !ready;
}

function paintAddBotJoined(server) {
  const bot = server && addBot.joined ? serverBots(server).find(b => b.member.id === addBot.joined) : null;
  $("add-bot-waiting").classList.toggle("hidden", Boolean(bot));
  $("add-bot-joined").classList.toggle("hidden", !bot);
  if (!bot) return;
  $("add-bot-joined-text").textContent = server.can_manage_roles
    ? `${bot.member.name} joined. Pick the roles it needs:`
    : `${bot.member.name} joined. Ask someone who manages roles to give it the roles it needs.`;
  $("add-bot-ability").textContent = bot.abilities;
  $("add-bot-roles").replaceWith(Object.assign(botRoleChips(server, bot.member, async () => {
    addBot.changed = true;
    paintAddBotJoined(await invoke("get_server", { serverId: server.id }));
  }), { id: "add-bot-roles" }));
}

function stopAddBotWatch() {
  clearInterval(addBot.timer);
  addBot.timer = null;
}

function addBotOpen() {
  return !$("modal-backdrop").classList.contains("hidden") && !$("modal-add-bot").classList.contains("hidden");
}

async function watchForBot() {
  if (!addBotOpen()) return stopAddBotWatch();
  let server;
  try {
    server = await invoke("get_server", { serverId: addBot.serverId });
  } catch {
    return;
  }
  const fresh = newBots(addBot.known, server);
  if (!fresh.length || !addBotOpen()) return;
  stopAddBotWatch();
  addBot.joined = fresh[0].member.id;
  addBot.changed = true;
  paintAddBotJoined(server);
}

async function openAddBot() {
  const server = state.currentServer;
  if (!server || !server.can_invite) return;
  stopAddBotWatch();
  Object.assign(addBot, { serverId: server.id, known: serverBots(server).map(b => b.member.id), invite: "", joined: null, changed: false });
  $("add-bot-name").value = "";
  $("add-bot-name").placeholder = BOT_DEFAULT_NAME;
  $("add-bot-port").open = false;
  paintAddBotInvite("Making an invite...", false);
  paintAddBotCommands();
  paintAddBotJoined(null);
  showModal("modal-add-bot");
  try {
    const invite = await invoke("get_invite", { serverId: server.id });
    if (addBot.serverId !== server.id || !addBotOpen()) return;
    addBot.invite = invite;
    paintAddBotInvite(invite, true);
    paintAddBotCommands();
    addBot.timer = setInterval(watchForBot, 2000);
  } catch (e) {
    paintAddBotInvite(String(e), false);
  }
}

async function closeAddBot() {
  stopAddBotWatch();
  hideModals();
  if (!addBot.changed || !state.currentServer || state.currentServer.id !== addBot.serverId) return;
  await refreshCurrentServer();
  if (!$("server-settings").classList.contains("hidden")) openServerSettings();
}

function wireBots() {
  $("bots-add").onclick = openAddBot;
  wireExternalLink($("bots-guide"), BOT_GUIDE_URL);
  wireExternalLink($("add-bot-guide"), BOT_GUIDE_URL);
  wireExternalLink($("add-bot-download"), BOT_DOWNLOAD_URL);
  wireExternalLink($("add-bot-porting"), BOT_PORTING_URL);
  $("add-bot-invite-copy").onclick = () => addBot.invite && copyText(addBot.invite, "Invite code copied");
  $("add-bot-name").addEventListener("input", paintAddBotCommands);
  $("add-bot-done").onclick = closeAddBot;
}

async function refreshCurrentServer() {
  const keepChannel = state.currentChannel;
  try {
    state.currentServer = await invoke("get_server", { serverId: state.currentServer.id });
  } catch {
    return;
  }
  renderServerRail();
  renderMembers();
  renderChannels();
  refreshVoiceInviteCards();
  renderVoicePanelActivity();
  syncRulesGate();
  syncAwaitedBar();
  syncAttachAccess();
  renderEvents();
  if (keepChannel) {
    const still = state.currentServer.channels.find(c => c.id === keepChannel.id);
    if (still) {
      state.currentChannel = still;
      syncPinButton();
      syncRulesGate();
      if (still.kind !== keepChannel.kind) {
        $("chat-hash").replaceChildren(channelHeaderMark(still));
        syncRulesGate();
      }
      renderChannelLog(await fetchChannelMessages(still));
      return;
    }
  }
  await selectChannel(state.currentServer.channels.find(c => c.kind === "text" && c.can_view !== false) || null);
}

function wireActivityPing() {
  const MIN_PING_GAP_MS = 30e3;
  let lastPing = 0;
  let pointerAt = null;
  const ping = () => {
    const now = Date.now();
    if (now - lastPing < MIN_PING_GAP_MS) return;
    lastPing = now;
    invoke("note_user_activity").catch(() => {});
  };
  const pointerMoved = e => {
    const at = `${e.screenX},${e.screenY}`;
    if (at === pointerAt) return;
    pointerAt = at;
    if (e.isTrusted) ping();
  };
  const trusted = e => {
    if (e.isTrusted) ping();
  };
  window.addEventListener("pointermove", pointerMoved, { passive: true });
  for (const evt of ["pointerdown", "keydown", "wheel"]) {
    window.addEventListener(evt, trusted, { passive: true });
  }
}

async function boot() {
  await markMobileApp();
  wireAccessibility();
  wireEvents();
  wireSaved();
  wirePhoneLayout();
  wireNarrowLayout();
  wireTimeoutModal();
  wireModerationModal();
  wireActivityPing();
  wireBots();
  if (IS_TAURI) {
    await invoke("ui_ready");
  } else {
    toast("Preview mode: running on mock data, not the real backend");
  }
  state.voice = await invoke("get_voice_settings");
  await loadSafety();
  await loadFriends();
  await loadOverlaySettings();
  loadUpdateStatus();
  loadSpeechVoices();
  await loadIntegrations();
  await loadAccount();
  if (!account.signed_in) {
    showSignIn();
    return;
  }
  if (account.needs_account_folder) {
    await openAccountFolder();
    return;
  }
  await continueBoot();
}

async function openAccountFolder() {
  try {
    account = await invoke("account_open_folder");
  } catch (e) {
    paintClaim(String(e));
    return;
  }
  if (!account.signed_in) showSignIn();
  else if (account.unclaimed_profile) showClaim();
  else await continueBoot();
}

function paintClaim(problem) {
  for (const id of ["claim-intro", "claim-profile", "claim-choices", "claim-note"]) $(id).classList.toggle("hidden", Boolean(problem));
  $("claim-retry-row").classList.toggle("hidden", !problem);
  $("claim-title").textContent = t(problem ? "claim.problem" : "claim.title");
  $("claim-error").textContent = problem || "";
  $("claim-error").classList.toggle("hidden", !problem);
  showModal("modal-claim");
}

function showClaim() {
  const email = (account.user && account.user.email) || "";
  $("claim-intro").textContent = t("claim.intro", { email });
  $("claim-profile").textContent = unclaimedProfileLine(account.unclaimed_profile, t);
  paintClaim("");
}

async function answerClaim(keep) {
  $("claim-keep").disabled = $("claim-fresh").disabled = true;
  try {
    account = await invoke("account_claim_profile", { keep });
    if (account.signed_in) await continueBoot();
    else showSignIn();
  } catch (e) {
    $("claim-error").textContent = String(e);
    $("claim-error").classList.remove("hidden");
  } finally {
    $("claim-keep").disabled = $("claim-fresh").disabled = false;
  }
}

setupTitlebar();

async function continueBoot() {
  try { claimUiState(localStorage, account.user && account.user.id); } catch {}
  state.user = await invoke("get_user");
  if (!state.user) {
    showModal("modal-username");
    const suggestion = ((account.user && account.user.name) || "").toLowerCase().replace(/[^a-z0-9_.]/g, "").slice(0, 32);
    if (suggestion && !$("username-input").value) $("username-input").value = suggestion;
    $("username-input").focus();
    return;
  }
  hideModals();
  renderUserPanel();
  await refreshServers();
  await loadReadState();
  await loadPins();
  await loadSaved();
  if (railServers().length) await selectServer(railServers()[0].id);
  else await enterDmMode();
  await primeFriendPresence();
  if (!(await takePendingInviteLink())) await offerTrayPrompt();
}

async function offerTrayPrompt() {
  let due = false;
  try { due = await invoke("tray_prompt_due"); } catch {}
  if (!due) return;
  let canStart = true;
  try { canStart = await invoke("autostart_allowed"); } catch {}
  $("tray-autostart").closest(".tray-opt").classList.toggle("hidden", !canStart);
  $("tray-autostart").checked = canStart;
  $("tray-close").checked = true;
  showModal("modal-tray");
}

$("tray-done").addEventListener("click", async () => {
  try {
    state.voice = await invoke("finish_tray_prompt", {
      autostart: $("tray-autostart").checked && !$("tray-autostart").closest(".tray-opt").classList.contains("hidden"),
      closeToTray: $("tray-close").checked,
    });
  } catch {}
  hideModals();
});

let account = { signed_in: false, server_host: "" };

async function loadAccount() {
  try {
    account = await invoke("account_status");
  } catch {}
  syncGifButton();
}

let signInUrl = null;

function syncServerNote() {
  const custom = $("signin-server-select").value !== "default";
  const note = $("signin-server-note");
  note.dataset.i18n = custom ? "signin.server.warning" : "signin.server.note";
  note.textContent = t(note.dataset.i18n);
  note.classList.toggle("warn", custom);
  note.classList.remove("error");
  $("signin-server-input").removeAttribute("aria-invalid");
}

function showServerProblem(key) {
  const note = $("signin-server-note");
  note.dataset.i18n = key;
  note.textContent = t(key);
  note.classList.remove("warn");
  note.classList.add("error");
  $("signin-server-input").setAttribute("aria-invalid", "true");
}

function isServerProblem(message) {
  return message.startsWith("signin.server.bad.");
}

function openServerEdit() {
  $("signin-server-select").classList.add("hidden");
  $("signin-server-edit").classList.remove("hidden");
  $("signin-server-input").value = "";
  syncServerNote();
  $("signin-server-input").focus();
}

function closeServerEdit() {
  const select = $("signin-server-select");
  select.value = account.is_default_server ? "default" : account.server;
  select.classList.remove("hidden");
  $("signin-server-edit").classList.add("hidden");
  syncServerNote();
  select.focus();
}

async function useAccountServer(server) {
  try {
    account = await invoke("set_account_server", { server });
    showSignIn();
  } catch (err) {
    const message = String(err);
    if (isServerProblem(message)) showServerProblem(message);
    else showSignInError(message);
  }
}

function showSignIn() {
  $("signin-start").classList.remove("hidden");
  $("signin-wait").classList.add("hidden");
  $("signin-error").classList.add("hidden");
  const select = $("signin-server-select");
  const custom = !account.is_default_server;
  const defaultHost = account.default_server_host || "commy.chat";
  const first = select.options[0];
  if (defaultHost === "commy.chat") first.dataset.i18n = "signin.server.default";
  else delete first.dataset.i18n;
  first.text = defaultHost === "commy.chat" ? t("signin.server.default") : t("signin.server.test", { host: defaultHost });
  if (custom && ![...select.options].some(o => o.value === account.server)) {
    select.insertBefore(new Option(account.server_host, account.server), select.options[1]);
  }
  select.value = custom ? account.server : "default";
  select.disabled = Boolean(account.server_from_env) || account.signed_in;
  select.title = account.server_from_env ? "Set by COMMY_AUTH_URL" : "";
  $("signin-back").classList.toggle("hidden", !account.signed_in);
  for (const [id, key] of [["signin-title", "signin.title"], ["signin-intro", "signin.intro"]]) {
    const node = $(id);
    node.dataset.i18n = account.signed_in ? key.replace("signin.", "signin.add.") : key;
    node.textContent = t(node.dataset.i18n);
  }
  select.classList.remove("hidden");
  $("signin-server-edit").classList.add("hidden");
  syncServerNote();
  if (!account.signed_in && account.pending_code) {
    signInUrl = account.pending_url;
    $("signin-code").textContent = account.pending_code;
    $("signin-start").classList.add("hidden");
    $("signin-wait").classList.remove("hidden");
  }
  showModal("modal-signin");
}

async function beginSignIn() {
  $("signin-error").classList.add("hidden");
  $("signin-go").disabled = true;
  try {
    const start = await invoke("account_begin_sign_in");
    signInUrl = start.verification_url;
    $("signin-code").textContent = start.user_code;
    $("signin-start").classList.add("hidden");
    $("signin-wait").classList.remove("hidden");
  } catch (e) {
    showSignInError(String(e));
  } finally {
    $("signin-go").disabled = false;
  }
}

function showSignInError(message) {
  $("signin-error").textContent = message;
  $("signin-error").classList.remove("hidden");
  $("signin-start").classList.remove("hidden");
  $("signin-wait").classList.add("hidden");
}

let addingAccount = false;
let accountChanges = Promise.resolve();

function onAccountChanged() {
  accountChanges = accountChanges.then(applyAccountChange, applyAccountChange);
  return accountChanges;
}

async function applyAccountChange() {
  const was = account.signed_in;
  await loadAccount();
  const step = accountChangeStep(was, account, addingAccount && !$("modal-signin").classList.contains("hidden"));
  if (step) addingAccount = false;
  if (step === "open") {
    toast(`Signed in as ${(account.user && account.user.name) || "you"}`);
    if (account.needs_account_folder) await openAccountFolder();
    else await continueBoot();
  } else if (step === "sign-in") {
    showSignIn();
  } else if (step === "accounts") {
    await openAccounts();
  }
  if (!$("user-settings").classList.contains("hidden")) renderAccountSettings();
}

function paintAccountBanner() {
  const strip = $("account-banner");
  strip.dataset.banner = "";
  paintProfileBanner(strip, state.user.banner, state.user.node, null, bannerColor(state.user.id));
  $("account-banner-upload").lastElementChild.textContent = state.user.banner ? "Change banner" : "Add banner";
  $("account-banner-remove").classList.toggle("hidden", !state.user.banner);
}

async function saveAccountBanner(data) {
  try {
    state.user = await invoke("set_profile_banner", { data });
    paintAccountBanner();
    renderProfilePopover();
    if (state.currentServer) await refreshCurrentServer();
    toast(data ? "Banner saved" : "Banner removed");
  } catch (err) {
    toast(String(err));
  }
}

async function openAccounts() {
  closeFloating();
  try {
    account = await invoke("account_status");
  } catch {}
  showAccountNote("");
  renderAccounts();
  showModal("modal-accounts");
}

function showAccountNote(message, isError = false) {
  const note = $("account-note");
  note.textContent = message || (canAddAccount(account) ? "" : t("accounts.limit", { count: account.max_accounts }));
  note.classList.toggle("error", Boolean(message) && isError);
}

function renderAccounts() {
  const list = $("account-list");
  list.replaceChildren();
  for (const row of accountRows(account)) {
    const name = row.name || row.email;
    const item = el("div", "account-row" + (row.current ? " current" : ""));
    item.setAttribute("role", "listitem");
    const pick = el("button", "account-pick");
    if (row.current) pick.setAttribute("aria-current", "true");
    const avatar = row.current && state.user ? makeAvatar(state.user.id, name, "avatar account-avatar") : el("div", "avatar account-avatar", initials(name || "?"));
    if (!(row.current && state.user)) avatar.style.setProperty("--tint", personTint(row.id));
    const text = el("span", "account-text");
    text.append(el("span", "account-name", name), el("span", "account-email", accountLine(row, account)));
    const check = el("span", "account-check");
    check.title = t("accounts.current");
    check.appendChild(svgIcon("check", 18));
    pick.append(avatar, text, check);
    pick.onclick = () => {
      if (!row.current) switchAccount(row);
    };
    const out = el("button", "btn-link account-sign-out", t("accounts.signOut"));
    out.onclick = () => (row.current ? signOutOfCurrent() : signOutOfOther(row));
    item.append(pick, out);
    list.appendChild(item);
  }
  $("account-add").disabled = !canAddAccount(account);
}

function setAccountsBusy(busy) {
  for (const button of $("modal-accounts").querySelectorAll("button")) button.disabled = busy;
  if (!busy) $("account-add").disabled = !canAddAccount(account);
}

async function switchAccount(row) {
  setAccountsBusy(true);
  try {
    account = await invoke("account_switch", { server: row.server, id: row.id });
  } catch (e) {
    try {
      account = await invoke("account_status");
    } catch {}
    renderAccounts();
    showAccountNote(String(e), true);
  } finally {
    setAccountsBusy(false);
  }
}

async function signOutOfCurrent() {
  if (!confirm(signOutPrompt(account, t))) return;
  try {
    account = await invoke("account_sign_out");
  } catch (e) {
    toast(String(e));
  }
}

async function signOutOfOther(row) {
  if (!confirm(t("accounts.signOut.other", { name: row.name || row.email }))) return;
  setAccountsBusy(true);
  try {
    account = await invoke("account_sign_out_other", { server: row.server, id: row.id });
    renderAccounts();
    showAccountNote("");
  } catch (e) {
    showAccountNote(String(e), true);
  } finally {
    setAccountsBusy(false);
  }
}

function renderAccountSettings() {
  const u = account.user || {};
  $("acct-name").textContent = u.name || "Signed out";
  $("acct-email").textContent = u.email || "";
  $("acct-server-line").textContent = `Account server: ${account.server_host}${account.server_from_env ? " (set by COMMY_AUTH_URL)" : ""}`;
  $("acct-device-line").textContent = account.device_linked
    ? "This device is linked to your account."
    : "This device is not linked yet; it links automatically on sign-in.";
  if (typeof refreshEntryStatus === "function") refreshEntryStatus();
}

boot().catch(e => toast(String(e)));

function closeDiscover() {
  if (!discoverState.open) return;
  discoverState.open = false;
  $("discover").classList.add("hidden");
  const button = $("discover-btn");
  button.classList.remove("active");
  button.setAttribute("aria-pressed", "false");
}

async function openDiscover() {
  if (discoverState.open) return;
  discoverState.open = true;
  $("discover").classList.remove("hidden");
  const button = $("discover-btn");
  button.classList.add("active");
  button.setAttribute("aria-pressed", "true");
  renderDiscoverChips();
  $("discover-search").focus();
  await loadDiscover(true);
}

function renderDiscoverChips() {
  const box = $("discover-chips");
  box.innerHTML = "";
  const chips = [{ id: null, label: "All" }, ...DIRECTORY_CATEGORIES];
  for (const chip of chips) {
    const button = el("button", "discover-chip", chip.label);
    button.type = "button";
    button.setAttribute("aria-pressed", String(discoverState.category === chip.id));
    button.onclick = () => {
      if (discoverState.category === chip.id) return;
      discoverState.category = chip.id;
      renderDiscoverChips();
      loadDiscover(true);
    };
    box.appendChild(button);
  }
}

function discoverFiltered() {
  return Boolean(normalizeDirectorySearch(discoverState.q) || discoverState.category);
}

async function loadDiscover(reset) {
  if (!reset && (discoverState.busy || !discoverState.cursor)) return;
  const request = ++discoverState.request;
  discoverState.busy = true;
  $("discover-more").disabled = true;
  if (reset && !discoverState.entries.length) renderDiscoverCards({ loading: true });
  let outcome = {};
  try {
    const page = await invoke("browse_directory", {
      q: normalizeDirectorySearch(discoverState.q) || null,
      category: discoverState.category,
      cursor: reset ? null : discoverState.cursor,
    });
    if (request !== discoverState.request) return;
    discoverState.entries = reset ? page.entries : discoverState.entries.concat(page.entries);
    discoverState.cursor = page.next_cursor || null;
    if (page.host) {
      discoverState.host = page.host;
      $("discover-sub").textContent = `Public servers listed by the people who run them, through ${page.host}.`;
    }
    outcome = { unavailable: page.available === false };
  } catch (e) {
    if (request !== discoverState.request) return;
    if (reset) {
      discoverState.entries = [];
      discoverState.cursor = null;
      outcome = { error: String(e) };
    } else {
      toast(String(e));
    }
  }
  discoverState.busy = false;
  $("discover-more").disabled = false;
  renderDiscoverCards(outcome);
}

function renderDiscoverCards(outcome = {}) {
  const grid = $("discover-grid");
  grid.innerHTML = "";
  for (const entry of discoverState.entries) grid.appendChild(discoverCard(entry));
  const empty = $("discover-empty");
  const none = !discoverState.entries.length;
  empty.classList.toggle("hidden", !none);
  if (none) {
    const view = discoverEmptyView({ ...outcome, host: discoverState.host, filtered: discoverFiltered() });
    $("discover-empty-art").replaceChildren(svgIcon(view.icon, 48));
    $("discover-empty-title").textContent = view.title;
    $("discover-empty-text").textContent = view.text;
    $("discover-retry").classList.toggle("hidden", !view.retry);
  }
  $("discover-more").classList.toggle("hidden", none || !discoverState.cursor);
}

function discoverCard(entry) {
  const card = el("div", "discover-card");
  card.appendChild(el("div", "discover-card-name", entry.name));
  const meta = el("div", "discover-card-meta");
  meta.appendChild(el("span", null, directoryCategoryLabel(entry.category)));
  meta.appendChild(el("span", "discover-dot"));
  meta.appendChild(el("span", null, memberBucketLabel(entry.members)));
  card.appendChild(meta);
  card.appendChild(el("div", "discover-card-text", directoryExcerpt(entry.description, 300)));
  const foot = el("div", "discover-card-foot");
  const join = el("button", "btn-primary btn-small", "Join");
  join.onclick = () => openInviteLink(entry.link);
  foot.appendChild(join);
  card.appendChild(foot);
  return card;
}

function fillDiscoveryCategories() {
  const select = $("discovery-category");
  if (select.options.length) return;
  for (const category of DIRECTORY_CATEGORIES) {
    const option = document.createElement("option");
    option.value = category.id;
    option.textContent = category.label;
    select.appendChild(option);
  }
}

function paintDiscoveryCounter() {
  const counter = directoryDescriptionCounter($("discovery-description").value);
  $("discovery-count").textContent = counter.text;
  $("discovery-count").classList.toggle("over", counter.over);
}

async function paintDiscoverySettings(server) {
  fillDiscoveryCategories();
  let listing = null;
  try {
    listing = await invoke("server_listing", { serverId: server.id });
  } catch {
    listing = null;
  }
  if (!state.currentServer || state.currentServer.id !== server.id) return;
  discoveryState.listing = listing;
  const view = discoverySettingsState({ server, listing });
  $("discovery-card").classList.toggle("readonly", !view.enabled);
  $("discovery-hint").textContent = view.hint;
  const description = $("discovery-description");
  const category = $("discovery-category");
  description.disabled = !view.enabled;
  category.disabled = !view.enabled;
  if (listing) {
    description.value = listing.description || "";
    category.value = listing.category;
  } else if (!discoveryState.dirty) {
    description.value = "";
    category.value = "other";
  }
  paintDiscoveryCounter();
  $("discovery-list").classList.toggle("hidden", view.listed);
  $("discovery-list").disabled = !view.enabled;
  $("discovery-update").classList.toggle("hidden", !view.listed);
  $("discovery-update").disabled = !view.enabled;
  $("discovery-unlist").classList.toggle("hidden", !view.listed);
  $("discovery-unlist").disabled = !view.enabled;
}

async function saveDiscoveryListing() {
  const server = state.currentServer;
  if (!server) return;
  const counter = directoryDescriptionCounter($("discovery-description").value);
  if (counter.over) return toast(`Keep the description under ${DIRECTORY_DESCRIPTION_MAX} characters`);
  const buttons = [$("discovery-list"), $("discovery-update")];
  buttons.forEach(b => { b.disabled = true; });
  try {
    await invoke("publish_listing", { serverId: server.id, description: $("discovery-description").value, category: $("discovery-category").value });
    discoveryState.dirty = false;
    discoverState.entries = [];
    toast(discoveryState.listing ? "Listing updated" : "Listed in Discover");
    await paintDiscoverySettings(server);
  } catch (e) {
    toast(String(e));
    buttons.forEach(b => { b.disabled = false; });
  }
}

async function removeDiscoveryListing() {
  const server = state.currentServer;
  if (!server) return;
  $("discovery-unlist").disabled = true;
  try {
    await invoke("unlist_server", { serverId: server.id });
    discoveryState.dirty = false;
    discoverState.entries = [];
    toast("Removed from Discover");
    await paintDiscoverySettings(server);
  } catch (e) {
    toast(String(e));
    $("discovery-unlist").disabled = false;
  }
}

function wireDiscover() {
  $("discover-btn").onclick = openDiscover;
  $("discover-search-icon").replaceChildren(svgIcon("search", 16));
  let timer = null;
  $("discover-search").addEventListener("input", e => {
    discoverState.q = e.target.value;
    clearTimeout(timer);
    timer = setTimeout(() => loadDiscover(true), 250);
  });
  $("discover-more").onclick = () => loadDiscover(false);
  $("discover-retry").onclick = () => loadDiscover(true);
  $("discovery-description").addEventListener("input", () => {
    discoveryState.dirty = true;
    paintDiscoveryCounter();
  });
  $("discovery-category").addEventListener("change", () => { discoveryState.dirty = true; });
  $("discovery-list").onclick = saveDiscoveryListing;
  $("discovery-update").onclick = saveDiscoveryListing;
  $("discovery-unlist").onclick = removeDiscoveryListing;
}

let voiceInviting = null;
function showVoiceCallMenuItems(channel) {
  const voice = channel.kind === "voice" && channel.can_view !== false;
  $("chm-voice-join").textContent = state.voiceChannelId === channel.id ? "Disconnect" : "Join Voice";
  $("chm-voice-join").classList.toggle("hidden", !voice);
  $("chm-voice-invite").classList.toggle("hidden", !voice);
}

function voiceInviteSource(serverId, channelId) {
  const current = state.currentServer && state.currentServer.id === serverId ? state.currentServer : null;
  const channel = current ? current.channels.find(c => c.id === channelId) : null;
  const connected = state.voiceServer && state.voiceServer.id === serverId && state.voiceChannelId === channelId ? state.voiceServer : null;
  const summary = state.servers.find(s => s.id === serverId);
  return {
    serverName: (current && current.name) || (connected && connected.name) || (summary && summary.name) || "",
    channelName: channel ? channelPlainName(channel) : (connected && connected.channelName) || "",
  };
}

async function openVoiceInvite(serverId, channelId) {
  voiceInviting = { server: serverId, channel: channelId, targets: null };
  $("voice-invite-where").textContent = voiceInviteTitle(voiceInviteSource(serverId, channelId));
  $("voice-invite-search").value = "";
  renderVoiceInviteList();
  showModal("modal-voice-invite");
  $("voice-invite-search").focus();
  const views = [];
  for (const summary of state.servers) {
    try {
      views.push(await invoke("get_server", { serverId: summary.id }));
    } catch {
      continue;
    }
  }
  let dms = [];
  try {
    dms = await invoke("list_dm_threads");
  } catch {}
  if (!voiceInviting || voiceInviting.server !== serverId || voiceInviting.channel !== channelId) return;
  voiceInviting.targets = { servers: views.map(s => ({ ...s, channels: s.channels.map(c => ({ ...c, name: channelPlainName(c) })) })), dms };
  renderVoiceInviteList();
}

function renderVoiceInviteList() {
  const list = $("voice-invite-list");
  list.replaceChildren();
  if (!voiceInviting || !voiceInviting.targets) {
    list.appendChild(el("div", "dm-list-empty", "Loading places to send to"));
    return;
  }
  const targets = forwardTargets({ ...voiceInviting.targets, query: $("voice-invite-search").value });
  for (const target of targets) {
    const row = el("button", "person-row forward-row");
    if (target.kind === "dm") row.appendChild(avatarWithPresence(target.name, target.user));
    else if (target.kind === "group") row.appendChild(groupAvatar());
    else row.appendChild(el("span", "server-chip", initials(target.sub)));
    const meta = el("div", "dm-meta");
    meta.appendChild(el("div", "dm-name", target.name));
    meta.appendChild(el("div", "dm-preview", target.sub));
    row.appendChild(meta);
    row.onclick = () => sendVoiceInvite(target);
    list.appendChild(row);
  }
  if (!targets.length) list.appendChild(el("div", "dm-list-empty", "Nowhere matches that name."));
}

async function sendVoiceInvite(target) {
  const inviting = voiceInviting;
  if (!inviting) return;
  try {
    if (target.kind === "dm") {
      await invoke("send_dm_voice_invite", { serverId: inviting.server, channelId: inviting.channel, userId: target.user });
    } else {
      await invoke("send_voice_invite", { serverId: inviting.server, channelId: inviting.channel, toServerId: target.server, toChannelId: target.channel });
    }
    hideModals();
    voiceInviting = null;
    toast(`Invite sent to ${target.name}`);
    if (target.kind !== "dm" && state.view === "server" && state.currentChannel && state.currentChannel.id === target.channel) await refreshChannelInPlace();
    else if (target.kind === "dm" && inDmView() && state.currentDm && state.currentDm.user === target.user) await selectDm(target.user, target.name);
  } catch (e) {
    toast(String(e));
  }
}

function voiceInviteLocalPresence(invite) {
  const server = state.currentServer;
  if (!server || server.id !== invite.server) return null;
  const channel = server.channels.find(c => c.id === invite.channel);
  if (!channel) return { known: true, gone: true };
  const ids = channel.voice_user_ids || [];
  return {
    known: true,
    server_name: server.name,
    channel_name: channelPlainName(channel),
    user_limit: channel.user_limit || 0,
    can_view: channel.can_view !== false,
    users: (channel.voice_users || []).map((name, i) => ({ id: ids[i] || null, name })),
  };
}

function renderVoiceInvite(message) {
  const invite = message.voice_invite;
  const card = el("div", "voice-invite");
  card.dataset.server = invite.server;
  card.dataset.channel = invite.channel;
  const body = el("div", "voice-invite-body");
  body.appendChild(el("div", "voice-invite-kicker", "Voice call invite"));
  body.appendChild(el("div", "voice-invite-title"));
  const row = el("div", "voice-invite-row");
  row.appendChild(el("div", "voice-invite-faces"));
  row.appendChild(el("span", "voice-invite-more empty"));
  row.appendChild(el("span", "voice-invite-status"));
  body.appendChild(row);
  card.appendChild(body);
  const join = el("button", "voice-invite-join", "Join");
  join.type = "button";
  card.appendChild(join);
  card.voiceInvite = invite;
  const local = voiceInviteLocalPresence(invite);
  paintVoiceInviteCard(card, invite, local);
  if (!local) {
    invoke("voice_channel_presence", { serverId: invite.server, channelId: invite.channel })
      .then(presence => paintVoiceInviteCard(card, invite, presence ? { known: true, ...presence } : { known: false }))
      .catch(() => paintVoiceInviteCard(card, invite, { known: true, gone: true }));
  }
  return card;
}

function paintVoiceInviteCard(card, invite, presence) {
  const live = presence && presence.known && !presence.gone ? presence : null;
  const inThisCall = Boolean(state.voiceServer) && state.voiceServer.id === invite.server && state.voiceChannelId === invite.channel;
  const users = live ? live.users || [] : [];
  const limit = live ? live.user_limit || 0 : 0;
  const full = live ? voiceInviteIsFull({ users, limit, inThisCall }) : false;
  card.querySelector(".voice-invite-title").textContent = voiceInviteTitle({ channelName: invite.channel_name, serverName: invite.server_name, live });
  const summary = voiceInviteSummary(users);
  const faces = card.querySelector(".voice-invite-faces");
  faces.replaceChildren(...summary.shown.map(user => makeAvatar(user.id, user.name, "avatar avatar-sm")));
  const more = card.querySelector(".voice-invite-more");
  more.textContent = summary.overflowLabel;
  more.classList.toggle("empty", !summary.overflow);
  more.title = summary.overflow ? `${summary.count} people are in the call` : "";
  const status = card.querySelector(".voice-invite-status");
  status.textContent = presence
    ? voiceInviteStatus({ known: presence.known, gone: presence.gone, canView: presence.can_view, users, limit })
    : "Looking up the call";
  const action = presence
    ? voiceInviteAction({ known: presence.known, gone: presence.gone, canView: presence.can_view, inThisCall, hasLink: Boolean(invite.invite_link), full })
    : { label: "Join", kind: "none", enabled: false, note: "" };
  const join = card.querySelector(".voice-invite-join");
  join.textContent = action.label;
  join.disabled = !action.enabled;
  join.title = action.note || "";
  join.onclick = e => {
    e.stopPropagation();
    if (action.kind === "join") joinVoiceInvite(invite);
    else if (action.kind === "link") openInviteLink(invite.invite_link);
  };
}

function refreshVoiceInviteCards() {
  for (const card of document.querySelectorAll(".voice-invite")) {
    const invite = card.voiceInvite;
    if (!invite) continue;
    const local = voiceInviteLocalPresence(invite);
    if (local) paintVoiceInviteCard(card, invite, local);
  }
}

async function joinVoiceInvite(invite) {
  if (!state.servers.some(s => s.id === invite.server)) {
    if (invite.invite_link) return openInviteLink(invite.invite_link);
    toast("Ask for an invite to this server");
    return;
  }
  if (!state.currentServer || state.currentServer.id !== invite.server) await selectServer(invite.server);
  const server = state.currentServer && state.currentServer.id === invite.server ? state.currentServer : null;
  const channel = server ? server.channels.find(c => c.id === invite.channel) : null;
  if (!channel) {
    toast("That voice channel is gone");
    return;
  }
  if (channel.can_view === false) {
    toast(`Only ${accessWho(server, channel.view_roles)} can join this channel`);
    return;
  }
  if (state.voiceChannelId === channel.id) return;
  if (voiceFull(channel)) {
    toast("That voice channel is full");
    return;
  }
  await toggleVoice(channel);
}

function renderVoiceInviteButton(call) {
  const button = $("vp-invite");
  setIcon(button, "friendAdd", 18);
  button.classList.toggle("hidden", call);
}

function wireVoiceInvites() {
  $("vp-invite").onclick = () => {
    const vs = state.voiceServer;
    if (!vs || vs.call || !state.voiceChannelId) return;
    openVoiceInvite(vs.id, state.voiceChannelId);
  };
  $("chm-voice-invite").onclick = () => {
    closeFloating();
    if (menuChannel && state.currentServer) openVoiceInvite(state.currentServer.id, menuChannel.id);
  };
  $("chm-voice-join").onclick = () => {
    closeFloating();
    const channel = menuChannel;
    if (!channel) return;
    if (voiceFull(channel)) {
      toast("That voice channel is full");
      return;
    }
    toggleVoice(channel);
  };
  $("voice-invite-cancel").onclick = hideModals;
  $("voice-invite-search").addEventListener("input", renderVoiceInviteList);
}
