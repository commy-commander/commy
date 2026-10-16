// Commy desktop frontend. Talks to the Rust side via Tauri invoke.
// When opened in a plain browser (no Tauri), a mock backend provides
// demo data so the UI can be developed and previewed standalone.

"use strict";

// Named IS_TAURI because Tauri's injected global API bundle already
// defines window.isTauri, and redeclaring it kills the whole script.
const IS_TAURI = Boolean(window.__TAURI__);
const invoke = IS_TAURI ? window.__TAURI__.core.invoke : mockInvoke;

// Surface any script failure instead of dying silently.
window.addEventListener("error", e => {
  try { toast(`UI error: ${e.message}`); } catch (_) { /* toast not ready */ }
});
window.addEventListener("unhandledrejection", e => {
  try { toast(`UI error: ${e.reason}`); } catch (_) { /* toast not ready */ }
});

const state = {
  user: null,
  servers: [],
  currentServer: null, // ServerView
  currentChannel: null, // ChannelView
  voice: { noise_cancellation: true, muted: false, deafened: false, output_volume: 1, input_gain: 1, afk_timeout_minutes: 10 },
  voiceChannelId: null, // voice channel we are connected to
  voiceServer: null, // { id, name, channelName } for the voice connection
  shareSource: null, // name of the monitor or window we are streaming
  watchingName: null,
  sharing: false, // we are sharing our screen
  watchingUser: null, // user id whose screen we are viewing
  view: "server", // "server" or "dm"
  currentDm: null, // { user, name }
  micTesting: false,
  people: new Map(), // user id -> person seen in any server
  speaking: new Set(), // user ids currently talking in our voice channel
  dmThreads: [],
  showDmProfile: true,
};

// ---------- Mock backend (browser preview only) ----------

const mock = {
  user: null,
  servers: {},
  messages: {},
  dms: {}, // partner id -> { name, messages }
  voice: { noise_cancellation: true, muted: false, deafened: false, output_volume: 1, input_gain: 1, afk_timeout_minutes: 10 },
};

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
      ch("rust-help", "text", "Borrow checker support group"),
      ch("General", "voice"),
    ],
    members: [
      { id: "m1", name: "ferris", presence: "online", custom_status: "heads down" },
      { id: "m2", name: "nova", presence: "idle", custom_status: null },
      { id: "m3", name: "sleepy", presence: "offline", custom_status: null },
    ],
  };
  server.roles = [
    { id: "r1", name: "Raider", color: "9b7bff", position: 2, permissions: [], self_assignable: true, tag: "RAID" },
    { id: "r2", name: "Moderator", color: "3ba55c", position: 3, permissions: ["manage_messages"], self_assignable: false, tag: null },
  ];
  server.game_tags = ["Rust", "Valorant"];
  server.rules = "# Welcome\n1. Be kind to each other\n2. **No spam** or self promotion\n3. Keep it on topic, see https://commy.chat";
  server.needs_rules = true;
  server.members[0].tag = "RAID";
  server.members[0].tag_color = "9b7bff";
  server.members[0].color = "9b7bff";
  mock.servers[server.id] = server;
  mock.dms.m2 = {
    name: "nova",
    kind: "request",
    messages: [
      { id: "d3", from: "m2", from_name: "nova", content: "hey, saw you in Rust Hangout. want to pair on the voice code?",
        created_at: new Date(Date.now() - 300e3).toISOString(), outgoing: false },
    ],
  };
  mock.reports = [{ id: "r1", server: server.id, channel: null, message: null, reporter: "m1", reporter_name: "ferris", reported: "m2", reported_name: "nova", reason: "spam", note: "Keeps posting invite links in every channel.", excerpt: "join my server https://example.com/invite", created_at: new Date().toISOString() }];
  mock.dms.m1 = {
    name: "ferris",
    messages: [
      { id: "d1", from: "m1", from_name: "ferris", content: "hey, did the voice fix land?",
        created_at: new Date(Date.now() - 26 * 3600e3).toISOString(), outgoing: false },
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
    { id: "x3", channel: c0, author_id: "me", author_name: mock.user ? mock.user.username : "you",
      content: "Posted while everyone was away, so it waits on this device.", created_at: new Date(Date.now() - 120e3).toISOString(), edited: false, attachments: [], pending: true },
    { id: "x4", channel: c0, author_id: "m2", author_name: "nova",
      content: "https://commy.chat/i/kq7m3n4p#" + "q".repeat(43), created_at: new Date(Date.now() - 60e3).toISOString(), edited: false, attachments: [] },
  ];
  // Doorbell: messages other members sent us that are still on their way.
  server.awaited = 2;
}

// Fake screen capture for the browser preview: a labeled gradient with a clock.
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

async function mockInvoke(cmd, args = {}) {
  switch (cmd) {
    case "get_user": return mock.user;
    case "init_user":
      mock.user = { id: "me", username: args.username, display_name: null, presence: "online", custom_status: null };
      if (Object.keys(mock.servers).length === 0) mockSeed();
      return mock.user;
    case "list_servers":
      return Object.values(mock.servers).map(s => ({ id: s.id, name: s.name, member_count: s.members.length, icon: s.icon || null }));
    case "get_server": {
      const s = mock.servers[args.serverId];
      if (!s) throw "unknown server";
      const members = [...s.members];
      if (mock.user && !members.some(m => m.id === "me")) {
        members.unshift({ id: "me", name: mock.user.username, presence: "online", custom_status: null });
      }
      return {
        ...s, members, screen_shares: [], roles: s.roles || [], afk_channel: s.afk_channel ?? null,
        game_tags: s.game_tags || [], events: s.events || [], rules: s.rules || null, needs_rules: Boolean(s.needs_rules), can_manage_events: true,
        channels: s.channels.map((c, i) => ({ parent: null, position: i, user_limit: 0, ...c })),
        can_manage_server: true, can_manage_channels: true, can_manage_roles: true, can_kick: true, can_manage_messages: true,
        emojis: s.emojis || [], sounds: s.sounds || [], emojis_shareable: !!s.emojis_shareable, soundboard_open: !!s.soundboard_open,
        can_manage_expressions: true, can_upload_sounds: true, can_use_soundboard: true,
        can_mute: true, can_ban: true, server_muted: s.server_muted || [], server_deafened: s.server_deafened || [], banned: s.banned || [],
      };
    }
    case "create_server": {
      const server = {
        id: crypto.randomUUID(), name: args.name, owner: "me",
        channels: [
          { id: crypto.randomUUID(), name: "general", kind: "text", topic: null, voice_users: [] },
          { id: crypto.randomUUID(), name: "General", kind: "voice", topic: null, voice_users: [] },
        ],
        members: [],
      };
      mock.servers[server.id] = server;
      return server;
    }
    case "join_server": throw "joining requires the desktop app";
    case "get_invite": return "commy" + "x".repeat(80) + " (mock invite, browser preview)";
    case "create_invite_link": return "https://commy.chat/i/kq7m3n4p#" + "q".repeat(43);
    case "resolve_invite_link": {
      const host = /commy:\/\/i\/([^/]+)\//.exec(args.link)?.[1] || new URL(args.link).host;
      return { server_name: "Pixel Pioneers", host, trusted: host === "commy.chat", account_server_host: "commy.chat",
        expires_at: new Date(Date.now() + 6 * 86400e3).toISOString(), ticket: "mock" };
    }
    case "get_messages": return mock.messages[args.channelId] || [];
    case "send_message": {
      const msg = {
        id: crypto.randomUUID(), channel: args.channelId, author_id: "me",
        author_name: mock.user.username, content: args.content,
        created_at: new Date().toISOString(), edited: false, attachments: [], reactions: [],
        reply_to: args.replyTo || null,
        reply_preview: (() => { const o = (mock.messages[args.channelId] || []).find(m => m.id === args.replyTo); return o ? { author_id: o.author_id, author_name: o.author_name, excerpt: o.content.slice(0, 120) } : null; })(),
      };
      (mock.messages[args.channelId] ||= []).push(msg);
      return msg;
    }
    case "get_voice_settings": return mock.voice;
    case "set_noise_cancellation": mock.voice.noise_cancellation = args.enabled; return mock.voice;
    case "set_muted": mock.voice.muted = args.muted; return mock.voice;
    case "set_deafened":
      mock.voice.deafened = args.deafened;
      if (args.deafened) mock.voice.muted = true;
      return mock.voice;
    case "set_voice_channel": return null;
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
        .sort((a, b) => b.last_at.localeCompare(a.last_at));
    case "get_dm_messages": return (mock.dms[args.userId] || { messages: [] }).messages;
    case "open_dm": {
      const member = Object.values(mock.servers).flatMap(s => s.members).find(m => m.id === args.userId);
      mock.dms[args.userId] ||= { name: member ? member.name : "Unknown User", messages: [] };
      return { user: args.userId, name: mock.dms[args.userId].name, last_message: "", last_at: "", kind: "inbox" };
    }
    case "send_dm": {
      const dm = mock.dms[args.userId];
      if (!dm) throw "no such conversation";
      // Offline partners get the message once they are back.
      const partner = Object.values(mock.servers).flatMap(s => s.members).find(m => m.id === args.userId);
      const sent = {
        id: crypto.randomUUID(), from: "me", from_name: mock.user.username, content: args.content,
        created_at: new Date().toISOString(), outgoing: true, pending: !partner || partner.presence === "offline",
      };
      dm.messages.push(sent);
      return sent;
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
        mock.avatars = { ...(mock.avatars || {}), [blob]: `data:image/jpeg;base64,${args.data}` };
        mock.user.avatar = `blob:${blob}`;
      } else {
        mock.user.avatar = args.kind === "emoji" ? `emoji:${args.data}` : null;
      }
      return mock.user;
    case "rename_server":
      mock.servers[args.serverId].name = args.name;
      return null;
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
    case "delete_channel": {
      const s = mock.servers[args.serverId];
      s.channels = s.channels.filter(c => c.id !== args.channelId);
      return null;
    }
    case "leave_server":
      delete mock.servers[args.serverId];
      return null;
    case "start_mic_test":
    case "stop_mic_test":
      return null;
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
      mock.servers[args.serverId][args.which] = ref;
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
    case "add_sound": {
      const s = mock.servers[args.serverId];
      const ref = `mock${Date.now()}@mock`;
      mock.expressions = { ...(mock.expressions || {}), [ref]: `data:audio/mpeg;base64,${args.data}` };
      s.sounds = (s.sounds || []).filter(x => x.name !== args.name).concat({ name: args.name, blob: ref, volume: args.volume ?? 1 });
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
    case "list_shareable_emojis":
      return Object.values(mock.servers).filter(s => s.emojis_shareable)
        .flatMap(s => (s.emojis || []).map(e => ({ server: s.id, server_name: s.name, name: e.name, image: e.image })));
    case "fetch_server_image": return (mock.serverImages || {})[args.image] || null;
    case "get_overlay_settings": return mock.overlay || (mock.overlay = { enabled: false, corner: "top-left", opacity: 0.9, show_chat: true, toggle_hotkey: "Ctrl+Shift+F10", chat_hotkey: "Ctrl+Shift+F11" });
    case "set_overlay_settings": mock.overlay = args.settings; return mock.overlay;
    case "overlay_hotkey_problems": return [];
    case "get_integrations": return mock.integrations || { igdb_client_id: "", igdb_configured: false };
    case "set_igdb_credentials":
      mock.integrations = { igdb_client_id: args.clientId, igdb_configured: Boolean(args.clientId) };
      return args.clientId ? [{ id: 72, name: "Portal 2", cover: "co1rs4", year: 2011 }] : [];
    case "gif_search": return null;
    case "search_games":
      return [{ id: 72, name: "Portal 2", cover: "co1rs4", year: 2011 }, { id: 71, name: "Portal", cover: "co1x7d", year: 2007 }]
        .filter(g => g.name.toLowerCase().includes(args.query.toLowerCase()));
    case "set_game_tags": mock.servers[args.serverId].game_tags = args.tags; mock.servers[args.serverId].game_tag_meta = args.meta || []; return null;
    case "set_rules": mock.servers[args.serverId].rules = args.rules; mock.servers[args.serverId].needs_rules = false; return null;
    case "accept_rules": mock.servers[args.serverId].needs_rules = false; return null;
    case "save_event": {
      const s = mock.servers[args.serverId];
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
    case "update_role": {
      const r = (mock.servers[args.serverId].roles || []).find(x => x.id === args.roleId);
      Object.assign(r, { name: args.name, color: args.color, self_assignable: args.selfAssignable, tag: args.tag });
      return null;
    }
    case "list_friends": return mock.friends || { friends: [{ id: "m1", name: "ferris" }], incoming: [{ id: "m2", name: "nova" }], outgoing: [] };
    case "friend_action": {
      const f = mock.friends || (mock.friends = { friends: [{ id: "m1", name: "ferris" }], incoming: [{ id: "m2", name: "nova" }], outgoing: [] });
      const name = (Object.values(mock.servers)[0].members.find(m => m.id === args.userId) || { name: "Someone" }).name;
      f.incoming = f.incoming.filter(x => x.id !== args.userId);
      if (args.action === "accept") f.friends.push({ id: args.userId, name });
      if (args.action === "request") f.outgoing.push({ id: args.userId, name });
      if (args.action === "remove") { f.friends = f.friends.filter(x => x.id !== args.userId); f.outgoing = f.outgoing.filter(x => x.id !== args.userId); }
      return null;
    }
    case "set_about": mock.user.about = args.about; return mock.user;
    case "set_server_afk_timeout": mock.servers[args.serverId].afk_timeout_minutes = args.minutes; return null;
    case "voice_stats":
      return { codec: "Opus, 48 kHz mono, 10 ms frames", bitrate_kbps: 32, links: [
        { user: "m1", name: "ferris", rtt_ms: 18 + Math.random() * 6, loss_pct: 0.1, jitter_ms: 1.4, path: "direct", tx_bytes: (mock.tx = (mock.tx || 0) + 150000), rx_bytes: (mock.rx = (mock.rx || 0) + 140000) },
        { user: "m2", name: "nova", rtt_ms: 140, loss_pct: 2.3, jitter_ms: 6.8, path: "relay", tx_bytes: 0, rx_bytes: 0 },
      ] };
    case "account_status": return mock.account || (mock.account = { signed_in: true, user: { id: "u1", name: "Commy Commander", email: "commy.cmd@commy.chat" }, server: "https://commy.chat", server_host: "commy.chat", is_default_server: true, default_server_host: "commy.chat", server_from_env: false, device_linked: true, pending_code: null, server_gifs: true, server_games: true, server_media: false });
    case "account_begin_sign_in": return { user_code: "WDJB-MJHT", verification_url: "https://commy.chat/device?user_code=WDJBMJHT", expires_in: 600 };
    case "account_cancel_sign_in": return null;
    case "account_sign_out": mock.account.signed_in = false; mock.account.user = null; return mock.account;
    case "set_account_server": return mock.account;
    case "set_user_voice": {
      mock.voice.user_volumes = mock.voice.user_volumes || {};
      mock.voice.muted_users = mock.voice.muted_users || [];
      if (args.volume !== null) mock.voice.user_volumes[args.userId] = args.volume;
      if (args.muted !== null) mock.voice.muted_users = args.muted ? [...mock.voice.muted_users, args.userId] : mock.voice.muted_users.filter(u => u !== args.userId);
      return mock.voice;
    }
    case "moderate_voice": {
      const s = mock.servers[args.serverId];
      s.server_muted = (s.server_muted || []).filter(u => u !== args.userId);
      s.server_deafened = (s.server_deafened || []).filter(u => u !== args.userId);
      if (args.muted) s.server_muted.push(args.userId);
      if (args.deafened) s.server_deafened.push(args.userId);
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
    case "get_safety": {
      const s = mock.safety || (mock.safety = { blocked: [], dm_policy: "everyone", spam_filter: true });
      return { ...s, blocked: s.blocked.map(id => ({ id, name: (Object.values(mock.servers)[0].members.find(m => m.id === id) || { name: "Someone" }).name })) };
    }
    case "set_blocked": {
      const s = mock.safety || (mock.safety = { blocked: [], dm_policy: "everyone", spam_filter: true });
      s.blocked = s.blocked.filter(id => id !== args.userId);
      if (args.blocked) s.blocked.push(args.userId);
      return null;
    }
    case "set_dm_settings": {
      const s = mock.safety || (mock.safety = { blocked: [], dm_policy: "everyone", spam_filter: true });
      s.dm_policy = args.policy;
      s.spam_filter = args.spamFilter;
      return null;
    }
    case "accept_dm_request": if (mock.dms[args.userId]) mock.dms[args.userId].kind = "inbox"; return null;
    case "decline_dm_request": delete mock.dms[args.userId]; return null;
    case "report_user": console.info("report", args); return null;
    case "list_reports": return mock.reports || [];
    case "dismiss_report": mock.reports = (mock.reports || []).filter(r => r.id !== args.reportId); return null;
    case "set_status_expiry":
      mock.voice.presence_until = args.presenceUntil;
      mock.voice.status_until = args.statusUntil;
      return mock.voice;
    case "set_unread": mock.unread = args.count; return null;
    case "get_awaited": return (mock.servers[args.serverId] || {}).awaited || 0;
    case "set_close_to_tray": mock.voice.close_to_tray = args.enabled; return mock.voice;
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
    case "start_screen_share": mock.share = { name: `Source ${args.sourceId}`, hue: args.sourceId * 47 }; return null;
    case "stop_screen_share": mock.share = null; return null;
    case "own_share_frame":
      if (!mock.share) throw "not sharing";
      return { width: 1280, height: 720, jpeg: mockFrame(mock.share.name, 640, 360, mock.share.hue), viewers: 2 };
    case "watch_screen":
      clearInterval(mock.watchTimer);
      mock.watchTimer = setInterval(() => {
        if (!state.watchingUser) return clearInterval(mock.watchTimer);
        showStageFrame(`data:image/jpeg;base64,${mockFrame("Remote stream", 960, 540, 200)}`, 1280, 720);
      }, 125);
      return null;
    case "stop_watching": clearInterval(mock.watchTimer); return null;
    case "create_role":
    case "delete_role":
    case "set_member_role":
    case "kick_member":
      return null;
    case "open_url":
      window.open(args.url, "_blank");
      return null;
    case "attach_file":
    case "download_attachment":
    case "note_user_activity": return null;
    case "get_attachment_preview":
      if (mock.avatars && mock.avatars[args.blob]) return mock.avatars[args.blob];
      throw "this feature needs the desktop app";
    default: throw `unknown command ${cmd}`;
  }
}

// ---------- DOM helpers ----------

const $ = id => document.getElementById(id);

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

// Inline SVG icons (Material Symbols paths, Apache 2.0).
const ICONS = {
  speaker: "M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8v8a4.5 4.5 0 0 0 2.5-4zM14 3.2v2.1a7 7 0 0 1 0 13.4v2.1a9 9 0 0 0 0-17.6z",
  mic: "M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.9V21h2v-3.1A7 7 0 0 0 19 11z",
  micOff: "M19 11h-1.7c0 .7-.2 1.4-.4 2l1.2 1.2c.6-.9.9-2 .9-3.2zm-4 .2V5a3 3 0 0 0-5.9-.8L15 10.1v1.1zM4.3 3 3 4.3l6 6V11a3 3 0 0 0 4.2 2.8l1.6 1.6A5 5 0 0 1 7 11H5a7 7 0 0 0 6 6.9V21h2v-3.1c.9-.1 1.8-.4 2.5-.9l4.2 4.2 1.3-1.3z",
  headset: "M12 3a9 9 0 0 0-9 9v7a2 2 0 0 0 2 2h3v-8H5v-1a7 7 0 0 1 14 0v1h-3v8h3a2 2 0 0 0 2-2v-7a9 9 0 0 0-9-9z",
  headsetOff: "M12 4a7 7 0 0 0-5.2 2.3L5.4 4.9A9 9 0 0 1 21 12v7c0 .3-.1.6-.2.9L19 18.1V14h-3v1.1L8.2 7.7A7 7 0 0 1 19 13v-1a7 7 0 0 0-7-7zM3.3 3 2 4.3l1.7 1.7A9 9 0 0 0 3 12v7a2 2 0 0 0 2 2h3v-8H5v-1c0-1.2.3-2.4.9-3.4L19.7 21 21 19.7z",
  chat: "M20 2H4a2 2 0 0 0-2 2v18l4-4h14a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2zm-2 12H6v-2h12zm0-3H6V9h12zm0-3H6V6h12z",
  plus: "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z",
  screen: "M20 3H4a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h5v2H7v2h10v-2h-2v-2h5a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zm0 13H4V5h16zm-9-2 5-3.5L11 7z",
  screenOff: "M21 16V5a2 2 0 0 0-2-2H6.8l2 2H19v10.2zM2.4 2.1 1.1 3.4 3 5.3V16a2 2 0 0 0 2 2h5v2H8v2h8v-2h-2v-2h3.7l3 3 1.3-1.3zM5 16V7.3l8.7 8.7z",
  hangup: "M12 9c-1.6 0-3.1.3-4.6.7v3.1c0 .4-.2.7-.6.9-1 .5-1.9 1.1-2.6 1.8a1 1 0 0 1-1.4 0L.3 13.1a1 1 0 0 1 0-1.4C3.3 8.8 7.5 7 12 7s8.7 1.8 11.7 4.7a1 1 0 0 1 0 1.4l-2.5 2.5a1 1 0 0 1-1.4 0c-.8-.7-1.6-1.3-2.6-1.8a1 1 0 0 1-.6-.9V9.7C15.1 9.3 13.6 9 12 9z",
  signal: "M2 22h3v-5H2zm5 0h3v-9H7zm5 0h3V9h-3zm5 0h3V2h-3z",
  noise: "M7 18h2V6H7zm4 4h2V2h-2zm-8-8h2v-4H3zm12 4h2V6h-2zm4-8v4h2v-4z",
  soundboard: "M12 3v10.55A4 4 0 1 0 14 17V7h4V3z",
  flag: "M14.4 6 14 4H5v17h2v-7h5.6l.4 2h7V6z",
  block: "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zM4 12a8 8 0 0 1 12.9-6.3L5.7 16.9A7.9 7.9 0 0 1 4 12zm8 8a7.9 7.9 0 0 1-4.9-1.7L18.3 7.1A8 8 0 0 1 12 20z",
  trash: "M6 19a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7H6zM19 4h-3.5l-1-1h-5l-1 1H5v2h14z",
  eyeOff: "M12 7a5 5 0 0 1 5 5 4.9 4.9 0 0 1-.36 1.83l2.92 2.92A11.8 11.8 0 0 0 22.99 12 11.83 11.83 0 0 0 11.99 4.5a11.6 11.6 0 0 0-4 .7l2.17 2.16A4.9 4.9 0 0 1 12 7zM2 4.27l2.28 2.28.46.46A11.8 11.8 0 0 0 1 12a11.82 11.82 0 0 0 15.4 6.66l.43.42L19.73 22 21 20.73 3.27 3zM7.53 9.8l1.55 1.55A2.8 2.8 0 0 0 9 12a3 3 0 0 0 3 3 2.8 2.8 0 0 0 .65-.08l1.55 1.55A5 5 0 0 1 7 12a4.9 4.9 0 0 1 .53-2.2zm4.31-.78 3.15 3.15.02-.16a3 3 0 0 0-3-3z",
  thread: "M4 4h16v12H5.17L4 17.17zm0-2a2 2 0 0 0-2 2v18l4-4h14a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2zm2 4h12v2H6zm0 4h9v2H6z",
  reply: "M10 9V5l-7 7 7 7v-4.1c5 0 8.5 1.6 11 5.1-1-5-4-10-11-11z",
  smilePlus: "M20 4V2h-2v2h-2v2h2v2h2V6h2V4zM12 20a8 8 0 0 1 0-16 7.9 7.9 0 0 1 2 .26V2.2A10 10 0 1 0 21.8 10h-2.06A7.9 7.9 0 0 1 20 12a8 8 0 0 1-8 8zm-3.5-9A1.5 1.5 0 1 0 7 9.5 1.5 1.5 0 0 0 8.5 11zm7 0A1.5 1.5 0 1 0 14 9.5 1.5 1.5 0 0 0 15.5 11zm-8 3a5 5 0 0 0 9 0z",
  play: "M8 5v14l11-7z",
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
  close: "M18.4 4.2 12 10.6 5.6 4.2 4.2 5.6 10.6 12l-6.4 6.4 1.4 1.4 6.4-6.4 6.4 6.4 1.4-1.4-6.4-6.4 6.4-6.4z",
  eye: "M12 5C7 5 2.7 8.1 1 12.5 2.7 16.9 7 20 12 20s9.3-3.1 11-7.5C21.3 8.1 17 5 12 5zm0 12.5a5 5 0 1 1 5-5 5 5 0 0 1-5 5zm0-8a3 3 0 1 0 3 3 3 3 0 0 0-3-3z",
  clock: "M12 2a10 10 0 1 0 10 10A10 10 0 0 0 12 2zm0 18a8 8 0 1 1 8-8 8 8 0 0 1-8 8zm.5-13H11v6l5.25 3.15.75-1.23-4.5-2.67z",
};

function svgIcon(name, size = 20) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", size);
  svg.setAttribute("height", size);
  svg.setAttribute("aria-hidden", "true");
  const path = document.createElementNS(ns, "path");
  path.setAttribute("fill", "currentColor");
  path.setAttribute("d", ICONS[name]);
  svg.appendChild(path);
  return svg;
}

function setIcon(button, name, size = 20) {
  button.replaceChildren(svgIcon(name, size));
}

function setVoiceStatus(node, name, title) {
  node.replaceChildren(svgIcon(name, 14));
  node.classList.add("off");
  node.title = title;
}

// ---------- Full-screen settings layers ----------

function openSettingsLayer(id, tab) {
  hideModals();
  for (const layer of document.querySelectorAll(".settings-layer")) layer.classList.add("hidden");
  $(id).classList.remove("hidden");
  selectSettingsTab(id, tab);
  for (const range of $(id).querySelectorAll('input[type="range"]')) syncRangeFill(range);
}

// Colors the slider track up to the thumb.
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
  }
  for (const page of layer.querySelectorAll(".settings-page")) {
    page.classList.toggle("hidden", page.dataset.page !== tab);
  }
  layer.querySelector(".settings-content").scrollTop = 0;
  layer.dataset.tab = tab;
}

function openSettingsLayerId() {
  const open = document.querySelector(".settings-layer:not(.hidden)");
  return open ? open.id : null;
}

async function closeSettingsLayers() {
  if (state.micTesting) await stopMicTest();
  stopCameraPreview();
  for (const layer of document.querySelectorAll(".settings-layer")) layer.classList.add("hidden");
}

// ---------- Appearance (stored per device) ----------

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
  try { localStorage.setItem(APPEARANCE_KEY, JSON.stringify(prefs)); } catch { /* storage unavailable */ }
}

function toast(message) {
  const node = $("toast");
  node.textContent = message;
  node.classList.remove("hidden");
  clearTimeout(node._timer);
  node._timer = setTimeout(() => node.classList.add("hidden"), 3000);
}

function showModal(id) {
  $("modal-backdrop").classList.remove("hidden");
  for (const modal of document.querySelectorAll(".modal")) modal.classList.add("hidden");
  $(id).classList.remove("hidden");
}

function hideModals() {
  $("modal-backdrop").classList.add("hidden");
}

// ---------- Avatars ----------

// user id -> { avatar, provider } from every server member list we load.
const avatarDirectory = new Map();
// blob hash -> data URL, or a pending promise.
const avatarImages = new Map();

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

function loadAvatarImage(blob, provider) {
  if (!avatarImages.has(blob)) {
    const pending = invoke("get_attachment_preview", { blob, provider, mime: "image/jpeg" })
      .then(url => {
        avatarImages.set(blob, url);
        return url;
      })
      .catch(() => {
        avatarImages.delete(blob);
        return null;
      });
    avatarImages.set(blob, pending);
  }
  return Promise.resolve(avatarImages.get(blob));
}

// Fills an avatar element with an image, emoji or initials, keeping
// any badges (presence dot) already inside it.
// Stable per-person tint so a room full of initials still reads as people.
function personTint(userId) {
  let hash = 0;
  for (const ch of String(userId)) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  return `hsl(${Math.abs(hash) % 360} 38% 44%)`;
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
      loadAvatarImage(blob, avatarFor(userId)?.provider).then(show);
    }
  } else {
    face = el("span", "avatar-face", initials(name || "?"));
  }
  node.prepend(face);
  return node;
}

// The line under a name: what they are playing, else emoji plus status.
function statusLine(person) {
  if (!person) return "";
  if (person.activity) return person.activity;
  return [person.status_emoji, person.custom_status].filter(Boolean).join(" ");
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

// "Today", "Yesterday", or the date, for day dividers and timestamps.
function dayLabel(date) {
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  if (date.toDateString() === today.toDateString()) return "Today";
  if (date.toDateString() === yesterday.toDateString()) return "Yesterday";
  const sameYear = date.getFullYear() === today.getFullYear();
  return date.toLocaleDateString([], sameYear ? { month: "long", day: "numeric" } : { year: "numeric", month: "long", day: "numeric" });
}

// ---------- Rendering ----------

// ---------- Server rail: order, folders, drag and drop ----------

// Per device: [{type:"server", id} | {type:"folder", id, name, open, servers:[id]}]
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
  try { localStorage.setItem(RAIL_KEY, JSON.stringify(layout)); } catch { /* session only */ }
}

// Drops servers we left, appends new ones, and tidies folders: empty
// folders vanish and a folder with one server unwraps.
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

// Applies one drop. `drag` is {kind:"server"|"folder", id}; `target` is
// {kind, id, folder?} for what was dropped on; `zone` is before|after|into.
function applyRailDrop(layout, drag, target, zone) {
  if (drag.id === target.id) return layout;
  if (drag.kind === "folder") {
    if (target.folder) return layout; // folders cannot nest
    const folder = layout.find(i => i.type === "folder" && i.id === drag.id);
    let rest = layout.filter(i => i !== folder);
    const at = rest.findIndex(i => i.id === target.id);
    if (!folder || at < 0) return layout;
    rest.splice(zone === "after" ? at + 1 : at, 0, folder);
    return rest;
  }
  let next = removeFromLayout(layout, drag.id);
  if (target.folder) {
    // Target sits inside a folder: reorder or join within it.
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
  return normalizeRailLayout(next, state.servers.map(s => s.id));
}

let railDrag = null;

function dropZone(e, node, allowInto) {
  const r = node.getBoundingClientRect();
  const x = (e.clientX - r.left) / r.width;
  if (!allowInto) return x < 0.5 ? "before" : "after";
  return x < 0.25 ? "before" : x > 0.75 ? "after" : "into";
}

function makeDropTarget(node, target) {
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
    const layout = applyRailDrop(normalizeRailLayout(loadRailLayout(), state.servers.map(s => s.id)), railDrag, target, zone);
    saveRailLayout(layout);
    railDrag = null;
    renderServerRail();
  });
}

function serverUnreadCount(serverId) {
  if (isServerMuted(serverId)) return 0;
  let total = 0;
  for (const [channel, server] of unread.channelServer) {
    if (server === serverId) total += unread.channels.get(channel) || 0;
  }
  return total;
}

function countBadge(count) {
  const badge = el("span", "count-badge", count > 99 ? "99+" : String(count));
  return badge;
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
  if (count) wrap.appendChild(countBadge(count));
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
  makeDropTarget(btn, { kind: "server", id: server.id, folder: folderId || null });
  return wrap;
}

function renderServerRail() {
  const list = $("server-list");
  list.innerHTML = "";
  const dmCount = [...unread.dms.values()].reduce((a, b) => a + b, 0);
  $("home-btn").classList.toggle("unread", dmCount > 0);
  $("home-badge")?.remove();
  if (dmCount) {
    const badge = countBadge(dmCount);
    badge.id = "home-badge";
    $("home-btn").after(badge);
  }
  const byId = new Map(state.servers.map(s => [s.id, s]));
  const layout = normalizeRailLayout(loadRailLayout(), state.servers.map(s => s.id));
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
        // Servers without an avatar still read as distinct colors.
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
    makeDropTarget(head, { kind: "folder", id: item.id, folder: null });
    folder.prepend(head);
    if (item.open) for (const s of servers) folder.appendChild(railServerButton(s, item.id));
    list.appendChild(folder);
  }
}

// ---------- Server and folder menus ----------

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
  // Rules and roles live in the full server view; fetch it for the menu.
  $("sm-rules").classList.add("hidden");
  $("sm-roles").classList.add("hidden");
  invoke("get_server", { serverId: server.id }).then(full => {
    $("sm-rules").classList.toggle("hidden", !full.rules);
    $("sm-roles").classList.toggle("hidden", !selfAssignableRoles(full).length);
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

function markServerRead(serverId) {
  for (const [channel, server] of [...unread.channelServer]) {
    if (server === serverId) unread.channels.delete(channel);
  }
  syncUnreadBadge();
  if (state.currentServer && state.currentServer.id === serverId) renderChannels();
}

let leavingServerId = null;

function openLeaveServer(serverId) {
  const server = state.servers.find(s => s.id === serverId);
  if (!server) return;
  leavingServerId = serverId;
  $("leave-server-title").textContent = `Leave '${server.name}'`;
  showModal("modal-leave-server");
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
      if (state.servers.length) await selectServer(state.servers[0].id);
      else await enterDmMode();
    }
  } catch (e) {
    toast(String(e));
  }
}

function renderChannels() {
  const list = $("channel-list");
  list.innerHTML = "";
  if (!state.currentServer) return;
  const server = state.currentServer;
  $("server-name").textContent = server.name;
  checkVoiceRoster();
  paintSidebarBanner();
  renderServerExtras(list, server);

  const canCreate = Boolean(server.can_manage_channels);
  const collapsed = collapsedSections();
  for (const section of channelSections(server, canCreate)) {
    const isCollapsed = collapsed.has(section.key);
    list.appendChild(sectionHeader(section, isCollapsed, canCreate));
    for (const channel of section.channels) {
      // Collapsed sections still show where you are.
      const active = (state.currentChannel && channel.id === state.currentChannel.id) || channel.id === state.voiceChannelId;
      if (isCollapsed && !active) continue;
      const row = channel.kind === "voice" ? voiceChannelRow(channel, canCreate) : textChannelRow(channel, canCreate);
      if (canCreate) {
        row.draggable = true;
        row.addEventListener("dragstart", e => {
          dragChannel = channel;
          e.dataTransfer.effectAllowed = "move";
        });
        makeDropTarget(row, () => dropChannel(section, channel.id));
      }
      list.appendChild(row);
      if (channel.kind === "voice") appendVoiceUsers(list, channel);
    }
  }
  pushOverlayRoster();
}

function textChannelRow(channel, canCreate) {
  const row = el("div", "channel");
  row.appendChild(el("span", "hash", "#"));
  row.appendChild(el("span", null, channel.name));
  if (state.currentChannel && channel.id === state.currentChannel.id) row.classList.add("active");
  if (unread.channels.get(channel.id)) row.classList.add("unread");
  row.onclick = () => selectChannel(channel);
  if (canCreate) row.appendChild(channelGear(channel));
  return row;
}

function voiceChannelRow(channel, canCreate) {
  const row = el("div", "channel voice-channel");
  const icon = el("span", "channel-icon");
  icon.appendChild(svgIcon("speaker", 18));
  row.appendChild(icon);
  row.appendChild(el("span", "channel-label", channel.name));
  const full = voiceFull(channel);
  if (channel.id === state.voiceChannelId) {
    row.classList.add("active", "connected");
    row.title = "Click to disconnect";
  } else {
    row.title = full ? "This channel is full" : "Click to join voice";
  }
  const count = voiceCountLabel(channel);
  if (count) {
    const badge = el("span", "voice-count" + (channel.user_limit ? " limited" : "") + (full ? " full" : ""), count);
    if (channel.user_limit) badge.title = full ? "Full" : `${channel.voice_users.length} of ${channel.user_limit} people`;
    row.appendChild(badge);
  }
  row.onclick = () => {
    if (full) {
      toast("That voice channel is full");
      return;
    }
    toggleVoice(channel);
  };
  if (canCreate) row.appendChild(channelGear(channel));
  return row;
}

function appendVoiceUsers(list, channel) {
  const myName = state.user ? (state.user.display_name || state.user.username) : null;
  const ids = channel.voice_user_ids || [];
  channel.voice_users.forEach((user, i) => {
    const userId = ids[i];
    const isMe = state.user && userId ? userId === state.user.id : user === myName;
    const vu = el("div", "voice-user");
    if (userId) {
      vu.dataset.user = userId;
      vu.classList.toggle("speaking", state.speaking.has(userId));
    }
    vu.appendChild(makeAvatar(userId, user, "avatar avatar-sm"));
    vu.appendChild(el("span", "voice-user-name", user));
    vu.appendChild(el("span", "vs-icon")).appendChild(svgIcon("mic", 14));
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
      vu.oncontextmenu = e => {
        e.preventDefault();
        e.stopPropagation();
        const member = server && server.members.find(m => m.id === userId);
        openMemberMenu(member || { id: userId, name: user }, e.clientX, e.clientY);
      };
    }
    // Only our own mic state is known locally; peers do not broadcast theirs yet.
    if (isMe && channel.id === state.voiceChannelId) {
      if (state.voice.muted) setVoiceStatus(vu.lastChild, "micOff", "Muted");
      if (state.voice.deafened) {
        vu.appendChild(el("span", "vs-icon"));
        setVoiceStatus(vu.lastChild, "headsetOff", "Deafened");
      }
    }
    list.appendChild(vu);
  });
}

// ---------- Server avatar and banner ----------

// image ref -> data URL, or a pending promise.
const serverImages = new Map();

function loadServerImage(ref) {
  if (!ref) return Promise.resolve(null);
  if (!serverImages.has(ref)) {
    const pending = invoke("fetch_server_image", { image: ref })
      .then(url => {
        serverImages.set(ref, url);
        return url;
      })
      .catch(() => {
        serverImages.delete(ref);
        return null;
      });
    serverImages.set(ref, pending);
  }
  return Promise.resolve(serverImages.get(ref));
}

// Stable per-server color for places that show many servers at once.
function serverTint(serverId) {
  let hash = 0;
  for (const ch of String(serverId)) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  return `hsl(${Math.abs(hash) % 360} 55% 45%)`;
}

// Shows a server's avatar on a rail button or settings preview.
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
  const banner = $("sidebar-banner");
  const ref = state.view === "server" && state.currentServer ? state.currentServer.banner : null;
  header.classList.toggle("has-banner", Boolean(ref));
  banner.style.backgroundImage = "";
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

// Lets the user pick an image. In the app this is the native dialog,
// because the webview's file input never opens a chooser in our Tauri
// window; the browser preview falls back to the file input.
function chooseImage(title, input) {
  if (IS_TAURI) return invoke("pick_image", { title });
  return new Promise(resolve => {
    input.value = "";
    input.onchange = () => resolve(input.files && input.files[0] ? URL.createObjectURL(input.files[0]) : null);
    input.click();
  });
}

// Center-crops an image file to the target size and encodes a JPEG.
function cropImageFile(file, width, height) {
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
      resolve(canvas.toDataURL("image/jpeg", 0.88).split(",")[1]);
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
    toast(data ? (which === "icon" ? "Server avatar updated" : "Banner updated") : "Removed");
    await refreshServers();
    await refreshCurrentServer();
    openServerSettings();
  } catch (e) {
    toast(String(e));
  }
}

// ---------- Sounds ----------

// Files bundled in the ui folder. "none" plays nothing.
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
  try { localStorage.setItem(SOUND_KEY, JSON.stringify(prefs)); } catch { /* session only */ }
}

function soundFor(eventId) {
  const event = SOUND_EVENTS.find(e => e.id === eventId);
  const chosen = soundPrefs().events[eventId];
  return chosen === undefined ? event?.fallback : chosen;
}

// Plays the sound for an event, unless sounds are off or you are on
// Do Not Disturb. `force` previews a specific sound from settings.
function playSound(eventId, force) {
  const prefs = soundPrefs();
  const name = force || soundFor(eventId);
  if (!name || name === "none" || !SOUND_FILES[name]) return;
  if (!force && (!prefs.enabled || state.user?.presence === "do_not_disturb")) return;
  const audio = new Audio(SOUND_FILES[name]);
  audio.volume = Math.max(0, Math.min(1, prefs.volume));
  audio.play().catch(() => { /* autoplay blocked before any user input */ });
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
      const opt = el("option", null, name === "none" ? "No sound" : name[0].toUpperCase() + name.slice(1));
      opt.value = name;
      select.appendChild(opt);
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

// Voice join and leave sounds: compare who is in our channel between renders.
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

// ---------- Notifications and unread ----------

const NOTIFY_KEY = "commy.notify";

function notifyPrefs() {
  try {
    return { level: "mentions", servers: {}, ...JSON.parse(localStorage.getItem(NOTIFY_KEY) || "{}") };
  } catch {
    return { level: "mentions", servers: {} };
  }
}

function saveNotifyPrefs(prefs) {
  try { localStorage.setItem(NOTIFY_KEY, JSON.stringify(prefs)); } catch { /* session only */ }
}

// channel id -> unread count, and DM partner id -> unread count.
const unread = { channels: new Map(), channelServer: new Map(), dms: new Map() };

function appFocused() {
  return document.visibilityState === "visible" && document.hasFocus();
}

function mentionsMe(content) {
  if (!state.user || !content) return false;
  // Whole names only, so "@Sam" does not ping "Samantha" and vice versa.
  const names = [state.user.username, state.user.display_name, "everyone", "here"].filter(Boolean)
    .map(n => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  return new RegExp(`@(${names.join("|")})(?![\\p{L}\\p{N}_])`, "iu").test(content);
}

// Pure decision so the rules are easy to reason about: returns true
// when a message should raise a desktop notification.
function shouldNotify({ level, presence, focusedOnIt, isDm, mentioned }) {
  if (presence === "do_not_disturb" || level === "none" || focusedOnIt) return false;
  if (isDm || level === "all") return true;
  // "online": everything, but only while your status is Online.
  if (level === "online") return presence === "online" || !presence;
  return level === "mentions" && mentioned;
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
  if (unread.channels.delete(channelId)) syncUnreadBadge();
}

function clearDmUnread(userId) {
  if (unread.dms.delete(userId)) syncUnreadBadge();
}

function serverHasUnread(serverId) {
  for (const [channel, server] of unread.channelServer) {
    if (server === serverId && unread.channels.get(channel)) return true;
  }
  return false;
}

function onMessageCreated(msg) {
  if (state.user && msg.author === state.user.id) return;
  if (isBlocked(msg.author)) return;
  const viewing = state.view === "server" && state.currentChannel && state.currentChannel.id === msg.channel;
  const focusedOnIt = viewing && appFocused();
  if (!focusedOnIt) {
    unread.channels.set(msg.channel, (unread.channels.get(msg.channel) || 0) + 1);
    unread.channelServer.set(msg.channel, msg.server);
    syncUnreadBadge();
    if (state.currentServer && state.currentServer.id === msg.server) renderChannels();
  }
  const prefs = notifyPrefs();
  const level = isServerMuted(msg.server) ? "none" : (prefs.servers[msg.server] && prefs.servers[msg.server] !== "default") ? prefs.servers[msg.server] : prefs.level;
  const body = plainText(msg.content) || (msg.has_attachments ? "Sent an attachment" : "");
  if (shouldNotify({ level, presence: state.user?.presence, focusedOnIt, isDm: false, mentioned: mentionsMe(msg.content) })) {
    invoke("notify", { title: `${msg.author_name} (#${msg.channel_name}, ${msg.server_name})`, body, target: `channel:${msg.server}:${msg.channel}` }).catch(() => {});
    playSound("message");
  }
}

// Messages that arrived in a catch-up with a peer only bump unread counts:
// no toast, no sound, no overlay line. Payload: { server, channels: { id: count } }.
function onMessagesCaughtUp(payload) {
  const server = payload && payload.server;
  const channels = payload && payload.channels;
  if (!server || !channels || typeof channels !== "object") return;
  let changed = false;
  for (const [channel, value] of Object.entries(channels)) {
    const count = Number(value);
    if (!Number.isInteger(count) || count <= 0) continue;
    const viewing = state.view === "server" && state.currentChannel && state.currentChannel.id === channel;
    if (viewing && appFocused()) continue;
    unread.channels.set(channel, (unread.channels.get(channel) || 0) + count);
    unread.channelServer.set(channel, server);
    changed = true;
  }
  if (!changed) return;
  syncUnreadBadge();
  if (state.currentServer && state.currentServer.id === server) renderChannels();
}

// A clicked notification opens the conversation it was about.
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
  } catch (e) {
    toast(String(e));
  }
}

function onDmReceived(dm) {
  if (dm.request) {
    // Requests wait quietly; spam is silent.
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
  if (shouldNotify({ level: prefs.level, presence: state.user?.presence, focusedOnIt, isDm: true, mentioned: true })) {
    invoke("notify", { title: dm.name, body: plainText(dm.content), target: `dm:${dm.user}` }).catch(() => {});
    playSound("dm");
  }
}

// ---------- Camera ----------

const CAMERA_KEY = "commy.cameraDevice";
const CAMERA_FPS = 12;
const cam = { stream: null, timer: null, busy: false, video: null, watching: new Set() };

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
    // A saved camera that was unplugged falls back to the default one.
    if (deviceId) return navigator.mediaDevices.getUserMedia({ video: true, audio: false });
    throw e;
  }
}

function blobToBase64(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).split(",")[1]);
    reader.onerror = reject;
    reader.readAsDataURL(blob);
  });
}

async function startCamera() {
  if (!state.voiceServer || cam.stream) return;
  try {
    cam.stream = await openCameraStream();
  } catch (e) {
    toast(`Camera unavailable: ${e.message || e}`);
    return;
  }
  try {
    await invoke("start_camera", { serverId: state.voiceServer.id });
  } catch (e) {
    stopCameraTracks();
    toast(String(e));
    return;
  }
  const video = document.createElement("video");
  video.muted = true;
  video.playsInline = true;
  video.srcObject = cam.stream;
  await video.play().catch(() => {});
  cam.video = video;
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  // Encode at most one frame at a time so a slow machine drops frames
  // instead of queueing them.
  cam.timer = setInterval(async () => {
    if (cam.busy || !video.videoWidth) return;
    cam.busy = true;
    try {
      const scale = Math.min(1, 640 / video.videoWidth);
      canvas.width = Math.round(video.videoWidth * scale);
      canvas.height = Math.round(video.videoHeight * scale);
      ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
      const blob = await new Promise(r => canvas.toBlob(r, "image/jpeg", 0.7));
      if (blob && cam.stream) {
        await invoke("push_camera_frame", {
          jpeg: await blobToBase64(blob),
          width: canvas.width,
          height: canvas.height,
        });
      }
    } catch { /* camera stopped mid-frame */ }
    cam.busy = false;
  }, Math.round(1000 / CAMERA_FPS));
  state.cameraOn = true;
  renderVoicePanel();
  if (state.currentServer) await refreshCurrentServer();
}

function stopCameraTracks() {
  clearInterval(cam.timer);
  cam.timer = null;
  if (cam.stream) for (const track of cam.stream.getTracks()) track.stop();
  cam.stream = null;
  cam.video = null;
}

async function stopCamera() {
  const server = state.voiceServer;
  stopCameraTracks();
  state.cameraOn = false;
  if (server) {
    try { await invoke("stop_camera", { serverId: server.id }); } catch { /* already off */ }
  }
  renderVoicePanel();
  if (state.currentServer) await refreshCurrentServer();
}

// Users with their camera on in our voice channel, ourselves first.
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
  // Stop pulling cameras that went away.
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
      if (isMe) {
        const video = document.createElement("video");
        video.muted = true;
        video.playsInline = true;
        video.autoplay = true;
        tile.appendChild(video);
      } else {
        tile.appendChild(el("img"));
        tile.appendChild(el("div", "cam-wait", "Connecting"));
      }
      tile.appendChild(el("div", "cam-name", isMe ? `${user.name} (you)` : user.name));
      grid.appendChild(tile);
    }
    tile.classList.toggle("speaking", state.speaking.has(user.id));
    if (isMe) {
      const video = tile.querySelector("video");
      if (cam.stream && video.srcObject !== cam.stream) video.srcObject = cam.stream;
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

// Settings: camera list needs permission once before labels appear.
async function loadCameraDevices() {
  const select = $("camera-device");
  try {
    const devices = (await navigator.mediaDevices.enumerateDevices()).filter(d => d.kind === "videoinput");
    const chosen = savedCameraId();
    select.replaceChildren();
    const def = el("option", null, "Default");
    def.value = "";
    select.appendChild(def);
    devices.forEach((d, i) => {
      const opt = el("option", null, d.label || `Camera ${i + 1}`);
      opt.value = d.deviceId;
      select.appendChild(opt);
    });
    select.value = devices.some(d => d.deviceId === chosen) ? chosen : "";
  } catch { /* no media devices API */ }
}

let previewStream = null;

async function toggleCameraPreview() {
  const video = $("cam-preview");
  if (previewStream) {
    stopCameraPreview();
    return;
  }
  try {
    previewStream = await openCameraStream();
    video.srcObject = previewStream;
    await video.play().catch(() => {});
    $("cam-preview-box").classList.add("live");
    $("cam-preview-btn").textContent = "Stop Test";
    loadCameraDevices();
  } catch (e) {
    toast(`Camera unavailable: ${e.message || e}`);
  }
}

function stopCameraPreview() {
  if (previewStream) for (const track of previewStream.getTracks()) track.stop();
  previewStream = null;
  $("cam-preview").srcObject = null;
  $("cam-preview-box").classList.remove("live");
  $("cam-preview-btn").textContent = "Test Camera";
}

// ---------- Audio devices ----------

function fillDeviceSelect(select, names, chosen) {
  select.replaceChildren();
  const def = el("option", null, "Default");
  def.value = "";
  select.appendChild(def);
  for (const name of names) {
    const opt = el("option", null, name);
    opt.value = name;
    select.appendChild(opt);
  }
  // A saved device that is unplugged stays visible so it is not lost.
  if (chosen && !names.includes(chosen)) {
    const opt = el("option", null, `${chosen} (not connected)`);
    opt.value = chosen;
    select.appendChild(opt);
  }
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

// ---------- Floating menus ----------

// Places a fixed-position popup next to an anchor, kept on screen.
function placePopup(popup, anchor, prefer = "above") {
  popup.classList.remove("hidden");
  // Anchors in the top bar open downward, whatever the caller prefers.
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

function closeFloating(except) {
  if (except !== "profile-pop") closeDurations();
  if (except !== "conn-pop" && connTimer) closeConnPop();
  for (const id of ["profile-pop", "avatar-menu", "emoji-pop", "server-menu", "folder-menu", "soundboard-pop", "member-menu", "member-card", "invite-menu", "audio-pop"]) {
    if (id !== except) $(id).classList.add("hidden");
  }
}

let emojiPopPick = null;
let emojiPopCustom = false;

// `custom`: also offer custom emojis, picked as "<:name:ref>" tokens.
function openEmojiPop(anchor, onPick, { custom = false } = {}) {
  emojiPopPick = onPick;
  emojiPopCustom = custom;
  $("emoji-pop-search").value = "";
  renderEmojiPop("");
  placePopup($("emoji-pop"), anchor, "above");
  $("emoji-pop-search").focus();
}

function renderEmojiPop(query) {
  const grid = $("emoji-pop-grid");
  grid.innerHTML = "";
  const q = query.trim().toLowerCase();
  if (emojiPopCustom) {
    appendCustomEmojiSections(grid, q, e => {
      $("emoji-pop").classList.add("hidden");
      if (emojiPopPick) emojiPopPick(`<:${e.name}:${e.image}>`);
    });
  }
  for (const [emoji, name] of EMOJI_DATA.filter(([, n]) => !q || n.includes(q)).slice(0, 160)) {
    const btn = el("button", null, emoji);
    btn.title = name;
    btn.onclick = () => {
      $("emoji-pop").classList.add("hidden");
      if (emojiPopPick) emojiPopPick(emoji);
    };
    grid.appendChild(btn);
  }
  if (!grid.childElementCount) grid.appendChild(el("div", "dm-preview", "No matches"));
}

// ---------- Own profile popover ----------

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
  $("pp-banner").style.background = bannerColor(user.id);
  const av = $("pp-avatar");
  av.replaceChildren();
  paintAvatar(av, user.id, name);
  av.appendChild(el("span", "presence-dot " + (user.presence || "online")));
  $("pp-name").textContent = name;
  $("pp-username").textContent = user.username;
  $("pp-status").value = user.custom_status || "";
  const emojiBtn = $("pp-emoji");
  if (user.status_emoji) emojiBtn.textContent = user.status_emoji;
  else emojiBtn.replaceChildren(svgIcon("smile", 20));
  emojiBtn.classList.toggle("set", Boolean(user.status_emoji));
  $("pp-clear").classList.toggle("hidden", !(user.custom_status || user.status_emoji));
  for (const btn of $("pp-presence").querySelectorAll("button")) {
    btn.classList.toggle("active", btn.dataset.presence === (user.presence || "online"));
  }
  renderStatusExpiry();
}

// Saves status fields while keeping everything else on the profile.
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

// ---------- Avatar picking ----------

let avatarMenuAnchor = null;

function openAvatarMenu(anchor) {
  avatarMenuAnchor = anchor;
  $("avatar-remove").classList.toggle("hidden", !(state.user && state.user.avatar));
  placePopup($("avatar-menu"), anchor, "below");
}

// Crops the picture to a centered square and re-encodes it as a
// 256 px JPEG so every peer downloads a small, uniform image.
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

// ---------- Channel settings ----------

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

function openChannelSettings(channelId) {
  editingChannelId = channelId;
  const channel = editingChannel();
  if (!channel) return;
  const voice = channel.kind === "voice";
  $("channel-settings-title").textContent = voice ? channel.name : `# ${channel.name}`;
  $("channel-edit-prefix").replaceChildren(voice ? svgIcon("speaker", 16) : document.createTextNode("#"));
  $("channel-edit-name").value = channel.name;
  const retention = String(channel.retention_days || 0);
  const select = $("channel-edit-retention");
  // Keep custom values from other clients selectable.
  if (![...select.options].some(o => o.value === retention)) {
    const opt = el("option", null, `${retention} days`);
    opt.value = retention;
    select.appendChild(opt);
  }
  select.value = retention;
  $("channel-limit-row").classList.toggle("hidden", !voice);
  $("channel-limit").value = String(channel.user_limit || 0);
  syncLimitLabel();
  syncChannelDirty();
  openSettingsLayer("channel-settings", "overview");
}

function channelEdits() {
  const channel = editingChannel();
  if (!channel) return null;
  let name = $("channel-edit-name").value.trim();
  if (channel.kind === "text" || channel.kind === "announcement") name = name.toLowerCase().replace(/\s+/g, "-");
  const retention = Number($("channel-edit-retention").value) || 0;
  const limit = channel.kind === "voice" ? Number($("channel-limit").value) || 0 : 0;
  const dirty = name !== channel.name || retention !== (channel.retention_days || 0) || limit !== (channel.user_limit || 0);
  return { channel, name, retention, limit, dirty };
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
    toast("Channel updated");
    await refreshCurrentServer();
    openChannelSettings(edits.channel.id);
  } catch (e) {
    toast(String(e));
  }
}

function syncLimitLabel() {
  const n = Number($("channel-limit").value) || 0;
  $("channel-limit-label").textContent = n ? `${n} ${n === 1 ? "person" : "people"}` : "No limit";
}

function openDeleteChannel() {
  const channel = editingChannel();
  if (!channel) return;
  const label = channel.kind === "voice" ? channel.name : `#${channel.name}`;
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
    toast(`Deleted ${channel.kind === "voice" ? channel.name : "#" + channel.name}`);
    editingChannelId = null;
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

// ---------- Create channel dialog ----------

let createChannelKind = "text";

let createChannelParent = null;

function openCreateChannel(kind, parent = null) {
  if (!state.currentServer || !state.currentServer.can_manage_channels) return;
  createChannelParent = parent;
  $("create-channel-name").value = "";
  setCreateChannelKind(kind);
  showModal("modal-create-channel");
  $("create-channel-name").focus();
}

function setCreateChannelKind(kind) {
  createChannelKind = kind;
  for (const opt of $("create-channel-kinds").querySelectorAll(".kind-option")) {
    opt.classList.toggle("active", opt.dataset.kind === kind);
  }
  $("create-channel-prefix").replaceChildren(
    kind === "voice" ? svgIcon("speaker", 16) : kind === "category" ? svgIcon("chevronDown", 16) : document.createTextNode("#"));
  $("create-channel-name").placeholder = kind === "voice" ? "Hangout" : kind === "category" ? "Games" : "new-channel";
  syncCreateChannelName();
}

// Text channel names are lowercase with dashes.
function syncCreateChannelName() {
  const input = $("create-channel-name");
  if (createChannelKind === "text") {
    const pos = input.selectionStart;
    const cleaned = input.value.toLowerCase().replace(/\s+/g, "-");
    if (cleaned !== input.value) {
      input.value = cleaned;
      input.setSelectionRange(pos, pos);
    }
  }
  $("create-channel-save").disabled = !input.value.trim();
}

async function saveCreateChannel() {
  const name = $("create-channel-name").value.trim();
  if (!name || !state.currentServer) return;
  try {
    if (createChannelKind === "category") {
      await invoke("create_category", { serverId: state.currentServer.id, name });
    } else {
      // A new channel inside a category stays there; a voice channel
      // created from a loose text section is loose too.
      await invoke("create_channel", { serverId: state.currentServer.id, name, kind: createChannelKind, parentId: createChannelParent });
    }
    hideModals();
    toast(`Created ${createChannelKind === "text" ? "#" : ""}${name}`);
    await refreshCurrentServer();
    if (createChannelKind === "text") {
      const made = state.currentServer.channels.filter(c => c.kind === "text").pop();
      if (made) await selectChannel(made);
    }
  } catch (e) {
    toast(String(e));
  }
}

// Speaking indicator: toggles the ring in place so it can update many
// times a second without re-rendering lists.
function setSpeaking(userId, speaking) {
  if (speaking) state.speaking.add(userId);
  else state.speaking.delete(userId);
  for (const node of document.querySelectorAll(`[data-user="${CSS.escape(userId)}"]`)) {
    node.classList.toggle("speaking", speaking);
  }
  if (state.user && userId === state.user.id) {
    document.querySelector(".user-panel").classList.toggle("speaking", speaking);
  }
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

  const online = state.currentServer.members.filter(m => m.presence !== "offline" && m.presence !== "invisible");
  const offline = state.currentServer.members.filter(m => m.presence === "offline" || m.presence === "invisible");

  const sharers = new Set((state.currentServer.screen_shares || []).map(s => s.user));

  const group = (title, members, dim) => {
    if (!members.length) return;
    list.appendChild(el("div", "member-group", `${title} - ${members.length}`));
    for (const member of members) {
      const row = el("div", "member" + (dim ? " offline" : ""));
      row.dataset.user = member.id;
      row.classList.toggle("speaking", state.speaking.has(member.id));
      const avatar = makeAvatar(member.id, member.name);
      const dot = el("span", "presence-dot " + member.presence);
      avatar.appendChild(dot);
      row.appendChild(avatar);
      const meta = el("div");
      const nameRow = el("div", "member-name", member.name);
      if (member.color) nameRow.style.color = `#${member.color}`;
      if (member.tag) nameRow.appendChild(roleTagChip(member.tag, member.tag_color));
      if (sharers.has(member.id)) {
        const live = el("span", "live-badge", "LIVE");
        live.title = "Watch screen";
        live.onclick = e => {
          e.stopPropagation();
          watchScreen(member.id, member.name);
        };
        nameRow.appendChild(live);
      }
      meta.appendChild(nameRow);
      const sub = statusLine(member);
      if (sub) {
        const line = el("div", "user-sub" + (member.activity ? " activity" : ""), sub);
        const cover = member.activity && igdbCover(member.activity_cover, "t_thumb");
        if (cover) {
          const img = el("img", "activity-cover");
          img.src = cover;
          img.alt = "";
          line.prepend(img);
        }
        meta.appendChild(line);
      }
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
  group("Online", online, false);
  group("Offline", offline, true);
}

function renderMessages(messages) {
  const box = $("messages");
  const viewKey = state.view === "dm" ? `dm:${state.currentDm && state.currentDm.user}` : `ch:${state.currentChannel && state.currentChannel.id}`;
  const sameView = box.dataset.view === viewKey;
  const prevScroll = box.scrollTop;
  const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 60;
  box.dataset.view = viewKey;
  box.innerHTML = "";
  if (state.view === "dm" && state.currentDm) box.appendChild(buildDmIntro());
  else if (state.currentChannel) box.appendChild(buildChannelIntro(state.currentChannel));
  if (!messages.length) return;
  state.renderedMessages = messages;
  const replyCounts = threadReplyCounts(messages);
  let lastAuthor = null;
  let lastTime = 0;
  let lastDay = null;
  let blockedRun = null;
  for (const message of messages) {
    const time = new Date(message.created_at).getTime();
    const day = new Date(message.created_at).toDateString();
    if (day !== lastDay) {
      box.appendChild(dayDivider(new Date(message.created_at)));
      lastDay = day;
      lastAuthor = null;
    }
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
    // A reply always shows its header so the quote reads clearly.
    const grouped = !message.reply_to && message.author_id === lastAuthor && time - lastTime < 5 * 60e3;
    box.appendChild(buildMessageRow(message, grouped, { replies: replyCounts.get(message.id) || 0 }));
    lastAuthor = message.author_id;
    lastTime = time;
  }
  // Stay put when re-rendering the same view while scrolled up.
  box.scrollTop = sameView && !atBottom ? prevScroll : box.scrollHeight;
  if (state.threadRoot) renderThread();
}

// The top of every channel: where you are and what it is for.
function buildChannelIntro(channel) {
  const intro = el("div", "channel-intro");
  const mark = el("div", "channel-intro-mark");
  if (channel.kind === "voice") mark.appendChild(svgIcon("speaker", 32));
  else mark.appendChild(el("span", "channel-intro-hash", "#"));
  intro.appendChild(mark);
  intro.appendChild(el("h2", null, `Welcome to #${channel.name}`));
  intro.appendChild(el("p", null, channel.topic || "This is the start of the channel. Say something."));
  return intro;
}

function dayDivider(date) {
  const row = el("div", "day-divider");
  row.appendChild(el("span", "day-divider-label", dayLabel(date)));
  return row;
}

// A lone GIF or image link renders as just the picture.
function isBareImageLink(content) {
  const text = (content || "").trim();
  if (findInviteLink(text)) return false;
  return /^https?:\/\/\S+$/i.test(text) && extractImageUrls(text).length === 1;
}

const PENDING_TIP = "Waiting for someone to come online";

// Clock shown on a message that has not reached anyone yet.
function pendingMark() {
  const mark = el("span", "msg-pending");
  mark.title = PENDING_TIP;
  mark.setAttribute("role", "img");
  mark.setAttribute("aria-label", PENDING_TIP);
  mark.appendChild(svgIcon("clock", 14));
  return mark;
}

function buildMessageRow(message, grouped, { replies = 0, inThread = false } = {}) {
  const pending = Boolean(message.pending);
  const row = el("div", "msg" + (grouped ? "" : " first") + (message.reply_to ? " is-reply" : "") + (pending ? " pending" : ""));
  row.dataset.id = message.id;
  row.appendChild(messageActions(message, inThread));
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
    if (pending) head.appendChild(pendingMark());
    body.appendChild(head);
  }
  // Grouped rows have no header, so the clock follows the text instead.
  let inlineMark = pending && grouped;
  if (message.content) {
    if (!isBareImageLink(message.content)) {
      const content = buildMessageContent(message.content);
      if (message.edited) content.appendChild(el("span", "edited", " (edited)"));
      if (inlineMark) content.appendChild(pendingMark());
      inlineMark = false;
      body.appendChild(content);
    }
    // Auto-embed pasted image and gif links.
    for (const url of extractImageUrls(message.content)) {
      body.appendChild(renderImageEmbed(url));
    }
  }
  if (inlineMark) body.prepend(pendingMark());
  for (const att of message.attachments || []) {
    body.appendChild(renderAttachment(att));
    if (att.is_image) body.appendChild(renderInlineImage(att));
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

// ---------- Replies and threads ----------

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

function jumpToMessage(id) {
  const row = $("messages").querySelector(`.msg[data-id="${CSS.escape(id)}"]`);
  if (!row) {
    toast("That message is not loaded");
    return;
  }
  row.scrollIntoView({ behavior: "smooth", block: "center" });
  row.classList.remove("flash");
  void row.offsetWidth;
  row.classList.add("flash");
}

// The thread root of a message: follow reply_to up while the parent is loaded.
function threadRootOf(message, byId) {
  let current = message;
  const seen = new Set();
  while (current.reply_to && byId.has(current.reply_to) && !seen.has(current.id)) {
    seen.add(current.id);
    current = byId.get(current.reply_to);
  }
  return current.id;
}

function threadReplyCounts(messages) {
  const byId = new Map(messages.map(m => [m.id, m]));
  const counts = new Map();
  for (const m of messages) {
    if (!m.reply_to) continue;
    const root = threadRootOf(m, byId);
    if (root !== m.id) counts.set(root, (counts.get(root) || 0) + 1);
  }
  return counts;
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

function openThread(rootId) {
  state.threadRoot = rootId;
  $("app").classList.add("thread-open");
  $("thread-panel").classList.remove("hidden");
  renderThread();
  $("thread-input").focus();
}

function closeThread() {
  state.threadRoot = null;
  $("app").classList.remove("thread-open");
  $("thread-panel").classList.add("hidden");
}

function renderThread() {
  const messages = state.renderedMessages || [];
  const byId = new Map(messages.map(m => [m.id, m]));
  const root = byId.get(state.threadRoot);
  const box = $("thread-messages");
  box.innerHTML = "";
  if (!root) {
    box.appendChild(el("div", "manage-empty", "This thread's first message is no longer loaded."));
    return;
  }
  const excerpt = plainText(root.content || "Attachment").slice(0, 60);
  $("thread-title").textContent = excerpt;
  const items = messages.filter(m => m.id === root.id || (m.reply_to && threadRootOf(m, byId) === root.id));
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
  try {
    await invoke("send_message", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      content: encodeCustomEmojis(content),
      replyTo: state.threadRoot,
    });
    await selectChannel(state.currentChannel);
  } catch (e) {
    input.value = content;
    toast(String(e));
  }
}

// ---------- Reactions ----------

function reactionPills(message) {
  const reactions = message.reactions || [];
  if (!reactions.length || state.view !== "server") return null;
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

function avatarNameFor(userId) {
  const member = state.currentServer && state.currentServer.members.find(m => m.id === userId);
  return member ? member.name : "Someone";
}

async function setReaction(message, emoji, active) {
  if (!state.currentServer || !state.currentChannel) return;
  try {
    await invoke("set_reaction", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      messageId: message.id,
      emoji,
      active,
    });
    // Keep the reader where they were instead of jumping to the bottom.
    const box = $("messages");
    const keep = box.scrollTop;
    await selectChannel(state.currentChannel);
    box.scrollTop = keep;
  } catch (e) {
    toast(String(e));
  }
}

// ---------- Message actions ----------

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

// Hiding is local to this device; nobody else is affected.
function setHidden(id, hidden) {
  const set = hiddenSet();
  if (hidden) set.add(id);
  else set.delete(id);
  try {
    // Cap growth: keep the most recent 2000 hides.
    localStorage.setItem(HIDDEN_KEY, JSON.stringify([...set].slice(-2000)));
  } catch { /* storage unavailable: hide lasts this session */ }
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
  if (state.view !== "server" || !state.currentServer) return false;
  return isOwnMessage(message) || Boolean(state.currentServer.can_manage_messages);
}

function messageActions(message, inThread = false) {
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
  if (state.view === "server") {
    add("smilePlus", "Add Reaction", e => openEmojiPop(e.currentTarget, emoji => setReaction(message, emoji, true), { custom: true }));
    if (inThread) {
      add("reply", "Reply", () => $("thread-input").focus());
    } else {
      add("reply", "Reply", () => startReply(message));
      add("thread", "Open Thread", () => openThread(message.id));
    }
  }
  if (state.view === "server" && isOwnMessage(message) && message.content) {
    add("pencil", "Edit", () => startEditMessage(message.id));
  }
  if (!isOwnMessage(message)) add("eyeOff", "Hide for Me", () => setHidden(message.id, true));
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
      // Shift-click skips the confirmation.
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
  input.value = decodeCustomEmojis(message.content);
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
    if (encodeCustomEmojis(text) === message.content) return finish();
    try {
      await invoke("edit_message", {
        serverId: state.currentServer.id,
        channelId: state.currentChannel.id,
        messageId: message.id,
        content: encodeCustomEmojis(text),
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

function renderAttachment(att) {
  const box = el("div", "attachment");
  const icon = el("span", "file-icon");
  icon.innerHTML = '<svg viewBox="0 0 24 24" width="28" height="28"><path fill="currentColor" d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8zm4 18H6V4h7v5h5z"/></svg>';
  box.appendChild(icon);
  const meta = el("div", "file-meta");
  meta.appendChild(el("div", "file-name", att.filename));
  meta.appendChild(el("div", "file-size", formatSize(att.size_bytes)));
  box.appendChild(meta);
  const btn = el("button", "btn-secondary btn-small", "Download");
  btn.onclick = async () => {
    btn.disabled = true;
    btn.textContent = "Fetching";
    try {
      const path = await invoke("download_attachment", {
        blob: att.blob,
        filename: att.filename,
        provider: att.provider ?? null,
      });
      toast(path ? `Saved to ${path}` : "Download cancelled");
    } catch (e) {
      toast(String(e));
    } finally {
      btn.disabled = false;
      btn.textContent = "Download";
    }
  };
  box.appendChild(btn);
  return box;
}

// Message text with URLs rendered as clickable links that open in
// the system browser. Built from text nodes, never innerHTML.
// ---------- Game overlay ----------

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

// Who is in our voice channel, as the overlay draws them.
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
      const member = server.members.find(m => m.id === id);
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
  // Keep the last roster while browsing another server.
  const roster = overlayRoster() || (state.voiceChannelId ? null : { members: [] });
  if (!roster) return;
  const key = JSON.stringify(roster);
  if (!force && key === lastOverlayRoster) return;
  lastOverlayRoster = key;
  emitToOverlay("overlay-roster", roster);
}

function pushOverlayChat(authorId, author, content, hasAttachments) {
  if (!overlaySettings || !overlaySettings.show_chat) return;
  const member = state.currentServer && state.currentServer.members.find(m => m.id === authorId);
  const text = plainText(content) || (hasAttachments ? "sent an attachment" : "");
  if (text) emitToOverlay("overlay-chat", { author, color: member && member.color, text: text.slice(0, 200) });
}

function renderOverlaySettings() {
  const s = overlaySettings;
  if (!s) return;
  $("overlay-enabled").checked = s.enabled;
  $("overlay-options").classList.toggle("disabled", !s.enabled);
  $("overlay-corner").value = s.corner;
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

// Turns a keydown into "Ctrl+Shift+O" style text.
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

function wireHotkeyInput(input, field) {
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
    const combo = hotkeyFromEvent(e);
    if (!combo) return;
    input.value = combo;
    input.dataset.prev = combo;
    input.blur();
    saveOverlaySettings({ [field]: combo });
  });
}

// ---------- Voice connection details ----------

let connTimer = null;
let connLast = null; // { at, tx, rx } for rates

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
  for (const link of stats.links) {
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
  // The signal icon follows the worst link.
  const worst = stats.links.reduce((w, l) => {
    const q = quality(l.rtt_ms, l.loss_pct);
    return q === "bad" || (q === "ok" && w === "good") ? q : w;
  }, "good");
  $("vp-signal").dataset.quality = worst;
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

// ---------- Profile bar quick audio ----------

// A short bounce, plus a slash that draws in when turning something off.
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
  pop.classList.remove("hidden");
  placePopup(pop, anchor, "above");
  try {
    const devices = await invoke("list_audio_devices");
    const names = input ? devices.inputs : devices.outputs;
    const current = (input ? state.voice.input_device : state.voice.output_device) || "";
    const def = el("option", null, "Default");
    def.value = "";
    select.replaceChildren(def, ...names.map(n => {
      const opt = el("option", null, n);
      opt.value = n;
      return opt;
    }));
    select.value = names.includes(current) ? current : "";
  } catch (e) {
    select.replaceChildren(el("option", null, "No devices found"));
  }
}

// ---------- Markdown (rules, event descriptions) ----------

// A small, safe subset: headings, lists, paragraphs, bold, italics,
// inline code and links. Builds DOM nodes; never uses innerHTML.
function renderMarkdown(text) {
  const root = el("div", "md");
  let list = null;
  for (const raw of String(text || "").split("\n")) {
    const line = raw.trimEnd();
    const heading = /^(#{1,3})\s+(.*)$/.exec(line);
    const bullet = /^\s*[-*]\s+(.*)$/.exec(line);
    const numbered = /^\s*(\d+)[.)]\s+(.*)$/.exec(line);
    if (bullet || numbered) {
      const tag = bullet ? "ul" : "ol";
      if (!list || list.tagName.toLowerCase() !== tag) {
        list = el(tag);
        if (numbered) list.start = Number(numbered[1]);
        root.appendChild(list);
      }
      const li = el("li");
      appendInlineMarkdown(li, bullet ? bullet[1] : numbered[2]);
      list.appendChild(li);
      continue;
    }
    list = null;
    if (!line.trim()) continue;
    const node = el(heading ? `h${heading[1].length + 2}` : "p");
    appendInlineMarkdown(node, heading ? heading[2] : line);
    root.appendChild(node);
  }
  return root;
}

function appendInlineMarkdown(node, text) {
  const re = /\*\*([^*]+)\*\*|\*([^*]+)\*|`([^`]+)`|(https?:\/\/[^\s<>"']+)/g;
  let last = 0;
  let m;
  while ((m = re.exec(text))) {
    if (m.index > last) node.appendChild(document.createTextNode(text.slice(last, m.index)));
    if (m[1]) node.appendChild(el("strong", null, m[1]));
    else if (m[2]) node.appendChild(el("em", null, m[2]));
    else if (m[3]) node.appendChild(el("code", null, m[3]));
    else {
      const url = m[4];
      const a = el("a", null, url);
      a.href = "#";
      a.onclick = e => {
        e.preventDefault();
        invoke("open_url", { url }).catch(err => toast(String(err)));
      };
      node.appendChild(a);
    }
    last = m.index + m[0].length;
  }
  if (last < text.length) node.appendChild(document.createTextNode(text.slice(last)));
}

// ---------- Images through the account server ----------

// GIF files and game covers load through the signed in account server, so
// the image hosts never see the user's address. The server keeps the same
// list of hosts (MEDIA_HOSTS in auth/src/media.js).
const MEDIA_HOSTS = new Set(["static.klipy.com", "images.igdb.com"]);

function viaAccountServer(url) {
  if (!account.server_media || !window.__TAURI__) return url;
  try {
    if (!MEDIA_HOSTS.has(new URL(url).hostname)) return url;
  } catch {
    return url;
  }
  // commy-media://localhost/ on macOS and Linux, http://commy-media.localhost/ on Windows
  return `${window.__TAURI__.core.convertFileSrc("", "commy-media")}?u=${encodeURIComponent(url)}`;
}

// ---------- IGDB ----------

let integrations = { igdb_client_id: "", igdb_configured: false };

async function loadIntegrations() {
  try {
    integrations = await invoke("get_integrations");
  } catch {
    /* keep defaults */
  }
}

function igdbCover(imageId, size = "t_cover_small") {
  return /^[A-Za-z0-9]{1,32}$/.test(imageId || "") ? viaAccountServer(`https://images.igdb.com/igdb/image/upload/${size}/${imageId}.jpg`) : null;
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

// Autocomplete under the game tag input, when IGDB is set up.
let gameSearchTimer = null;
let gameSearchSeq = 0;

function closeGameSearch() {
  $("game-search").classList.add("hidden");
  $("game-search").replaceChildren();
}

function queueGameSearch(query) {
  clearTimeout(gameSearchTimer);
  // Own credentials or the account server's; with neither it finds nothing.
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

// ---------- Game tags ----------

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
    const chip = el("span", "game-chip", tag);
    const cover = state.currentServer && tagCover(state.currentServer, tag, "t_thumb");
    if (cover) {
      const img = el("img", "game-chip-cover");
      img.src = cover;
      chip.prepend(img);
    }
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

// Chips, Events and Rules shortcuts at the top of the channel list.
function renderServerExtras(list, server) {
  if (server.game_tags && server.game_tags.length) {
    const row = el("div", "sidebar-game-tags");
    for (const tag of server.game_tags) {
      const chip = el("span", "game-chip small", tag);
      const cover = tagCover(server, tag, "t_thumb");
      if (cover) {
        const img = el("img", "game-chip-cover");
        img.src = cover;
        chip.prepend(img);
      }
      row.appendChild(chip);
    }
    list.appendChild(row);
  }
  const now = Date.now();
  const upcoming = (server.events || []).filter(e => new Date(e.end || e.start).getTime() >= now);
  const events = el("div", "channel sidebar-link");
  events.appendChild(el("span", "channel-icon")).appendChild(svgIcon("calendar", 18));
  events.appendChild(el("span", "channel-label", upcoming.length ? `${upcoming.length} Event${upcoming.length === 1 ? "" : "s"}` : "Events"));
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

// ---------- Rules ----------

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
    if (selfAssignableRoles(state.currentServer).length) openServerWelcome("roles");
    else {
      hideModals();
      toast("Thanks! You can chat now");
    }
  } catch (e) {
    toast(String(e));
  }
}

// One panel for joining a server: read the rules, then pick roles, without
// a second dialog popping up behind the first.
function openServerWelcome(step) {
  const server = state.currentServer;
  if (!server) return;
  closeFloating();
  const hasRoles = selfAssignableRoles(server).length > 0;
  const twoSteps = Boolean(server.rules) && hasRoles;
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

// Servers we just joined that should offer the role picker once.
const rolePromptPending = new Set();

function maybePromptRoles() {
  const server = state.currentServer;
  if (!server || !rolePromptPending.has(server.id) || server.needs_rules) return;
  if (!selfAssignableRoles(server).length) return;
  if (!$("modal-backdrop").classList.contains("hidden")) return;
  rolePromptPending.delete(server.id);
  openServerWelcome("roles");
}

// The composer is replaced by an accept prompt until the rules are accepted.
function syncRulesGate() {
  const gated = state.view === "server" && Boolean(state.currentServer && state.currentServer.needs_rules);
  $("rules-gate").classList.toggle("hidden", !gated);
  if (state.view === "server") document.querySelector(".composer").classList.toggle("hidden", gated);
  if (gated && !rulesPrompted.has(state.currentServer.id)) {
    rulesPrompted.add(state.currentServer.id);
    openServerWelcome("rules");
  }
  if (!gated) maybePromptRoles();
}

// Doorbell: messages someone sent us that are still on their way.
function awaitedText(count) {
  const n = Number(count);
  if (!Number.isInteger(n) || n <= 0) return null;
  return n === 1
    ? "1 message is on its way. It arrives when the sender's device is online."
    : `${n} messages are on their way. They arrive when the sender's device is online.`;
}

let awaitedSeq = 0;

async function syncAwaitedBar() {
  const seq = ++awaitedSeq;
  const serverId = state.view === "server" && state.currentServer ? state.currentServer.id : null;
  let text = null;
  if (serverId) {
    try {
      text = awaitedText(await invoke("get_awaited", { serverId }));
    } catch {
      // Older backends lack the command; keep the bar hidden.
    }
  }
  // A newer call or a server switch wins over this answer.
  if (seq !== awaitedSeq) return;
  $("awaited-text").textContent = text || "";
  $("awaited-bar").classList.toggle("hidden", !text);
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

// ---------- Events ----------

const eventsView = { mode: "upcoming", month: null, day: null };
let editingEventId = null;

function openEvents() {
  if (!state.currentServer) return;
  closeFloating();
  const first = new Date();
  eventsView.month = new Date(first.getFullYear(), first.getMonth(), 1);
  eventsView.day = null;
  $("events-title").textContent = `${state.currentServer.name} Events`;
  $("event-create").classList.toggle("hidden", !state.currentServer.can_manage_events);
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
    empty.appendChild(el("p", null, server.can_manage_events ? "Schedule one so people know when to show up." : "Check back later."));
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

function openEventEditor(event) {
  const server = state.currentServer;
  editingEventId = event ? event.id : null;
  $("event-editor-title").textContent = event ? "Edit Event" : "Create Event";
  $("event-title").value = event ? event.title : "";
  $("event-description").value = event ? event.description : "";
  const start = event ? new Date(event.start) : new Date(Math.ceil(Date.now() / 3600e3) * 3600e3 + 3600e3);
  $("event-start").value = toLocalInput(start);
  $("event-end").value = event && event.end ? toLocalInput(new Date(event.end)) : "";
  const select = $("event-channel");
  select.replaceChildren();
  const none = el("option", null, "No voice channel");
  none.value = "";
  select.appendChild(none);
  for (const channel of server.channels.filter(c => c.kind === "voice")) {
    const opt = el("option", null, channel.name);
    opt.value = channel.id;
    select.appendChild(opt);
  }
  select.value = event && event.channel ? event.channel : "";
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

// ---------- Channel categories ----------

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
  try { localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...set])); } catch { /* session only */ }
  renderChannels();
}

// Sidebar sections: loose text and voice channels, then each category.
function channelSections(server, canCreate) {
  const byPos = (a, b) => (a.kind === "voice") - (b.kind === "voice") || a.position - b.position;
  const categories = server.channels.filter(c => c.kind === "category").sort((a, b) => a.position - b.position);
  const catIds = new Set(categories.map(c => c.id));
  const loose = server.channels.filter(c => c.kind !== "category" && !(c.parent && catIds.has(c.parent)));
  const isText = c => c.kind === "text" || c.kind === "announcement";
  const sections = [];
  const looseText = loose.filter(isText).sort(byPos);
  const looseVoice = loose.filter(c => c.kind === "voice").sort(byPos);
  if (looseText.length || canCreate) sections.push({ key: `${server.id}:text`, title: "Text Channels", kind: "text", parent: null, channels: looseText });
  if (looseVoice.length || canCreate) sections.push({ key: `${server.id}:voice`, title: "Voice Channels", kind: "voice", parent: null, channels: looseVoice });
  for (const category of categories) {
    sections.push({
      key: category.id,
      title: category.name,
      kind: "text",
      parent: category.id,
      category,
      channels: server.channels.filter(c => c.parent === category.id && c.kind !== "category").sort(byPos),
    });
  }
  return sections;
}

let dragChannel = null;

// Drops a dragged channel into `section`, before `beforeId` or at the end.
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
  // Categories sit after loose channels; keep their positions high.
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

// Voice channel member counts against the limit.
function voiceCountLabel(channel) {
  const n = channel.voice_users.length;
  if (channel.user_limit) return `${String(n).padStart(2, "0")}/${String(channel.user_limit).padStart(2, "0")}`;
  return n ? String(n) : "";
}

function voiceFull(channel) {
  return Boolean(channel.user_limit) && channel.voice_users.length >= channel.user_limit
    && channel.id !== state.voiceChannelId && !(state.currentServer && state.currentServer.can_manage_channels);
}

// ---------- Roles: tags and self-assign ----------

function selfAssignableRoles(server) {
  return (server && server.roles || []).filter(r => r.self_assignable);
}

function isEmojiTag(tag) {
  return !/[A-Za-z0-9]/.test(tag) && [...tag].length <= 4;
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
    chip.style.color = `#${color}`;
    chip.style.borderColor = `#${color}66`;
  }
  return chip;
}

// Paints a banner ref (or a stable color) onto an element.
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

// ---------- Friends ----------

let friends = { friends: [], incoming: [], outgoing: [] };

async function loadFriends() {
  try {
    friends = await invoke("list_friends");
  } catch {
    /* keep the last list */
  }
}

function friendState(userId) {
  if (friends.friends.some(f => f.id === userId)) return "friend";
  if (friends.incoming.some(f => f.id === userId)) return "incoming";
  if (friends.outgoing.some(f => f.id === userId)) return "outgoing";
  return "none";
}

async function friendAction(userId, action, name) {
  try {
    await invoke("friend_action", { userId, action });
    await loadFriends();
    const done = { request: `Friend request sent to ${name}`, accept: `You and ${name} are now friends`, decline: "Request declined", remove: `Removed ${name} from friends` };
    toast(done[action] || "Done");
    if (state.view === "dm") renderDmThreads();
  } catch (e) {
    toast(String(e));
  }
}

// One friend button whose label follows the current state.
function friendButton(person, onDone) {
  const st = friendState(person.id);
  const labels = { none: "Add Friend", outgoing: "Request Sent", incoming: "Accept Friend", friend: "Friends" };
  const btn = el("button", "mc-action" + (st === "friend" ? " on" : ""));
  btn.appendChild(svgIcon(st === "friend" ? "check" : "friendAdd", 16));
  btn.appendChild(el("span", null, labels[st]));
  btn.title = st === "friend" ? "Remove friend" : st === "outgoing" ? "Cancel request" : labels[st];
  btn.onclick = async () => {
    const action = st === "none" ? "request" : st === "incoming" ? "accept" : "remove";
    if (action === "remove" && st === "friend" && !confirm(`Remove ${person.name} from your friends?`)) return;
    await friendAction(person.id, action, person.name);
    if (onDone) onDone();
  };
  return btn;
}

// Servers you are in that this person is not, for Invite to Server.
function invitableServers(userId) {
  const person = state.people.get(userId);
  const shared = new Set((person ? person.servers : []).map(s => s.id));
  return state.servers.filter(s => !shared.has(s.id));
}

function openInviteMenu(person, anchor) {
  const menu = $("invite-menu");
  menu.replaceChildren(el("div", "ctx-title", `Invite ${person.name} to`));
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

// Server invite tickets inside messages get a Join card.
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
      rolePromptPending.add(summary.id);
      await refreshServers();
      await selectServer(summary.id);
      toast("Joined server, syncing from peers");
    } catch (e) {
      join.disabled = false;
      toast(String(e));
    }
  };
  card.appendChild(join);
  return card;
}

// ---------- Full profile ----------

function openFullProfile(member) {
  closeFloating();
  const person = { ...(state.people.get(member.id) || {}), ...member };
  paintBanner($("fp-banner"), person.role_banner, person.color ? roleGradient(person.color) : bannerColor(person.id));
  const av = $("fp-avatar");
  av.replaceChildren();
  paintAvatar(av, person.id, person.name);
  av.appendChild(el("span", "presence-dot " + (person.presence || "offline")));
  $("fp-name").textContent = person.name;
  $("fp-name").style.color = person.color ? `#${person.color}` : "";
  $("fp-name").parentElement.querySelector(".role-tag")?.remove();
  if (person.tag) $("fp-name").after(roleTagChip(person.tag, person.tag_color));
  $("fp-username").textContent = person.username ? `@${person.username}` : "";
  $("fp-status").textContent = statusLine(person) || PRESENCE_LABELS[person.presence] || "";
  $("fp-about-wrap").classList.toggle("hidden", !person.about);
  $("fp-about").textContent = person.about || "";
  const servers = (state.people.get(person.id) || {}).servers || [];
  $("fp-mutual-label").textContent = `Mutual Servers - ${servers.length}`;
  $("fp-servers").replaceChildren(...servers.map(s => {
    const row = el("button", "dm-profile-server");
    row.appendChild(el("span", "server-chip", initials(s.name)));
    row.appendChild(el("span", null, s.name));
    row.onclick = () => {
      hideModals();
      selectServer(s.id);
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
    buttons.append(friendButton(person, () => openFullProfile(member)), msg);
  }
  showModal("modal-profile");
}

// ---------- Member profile card ----------

// Where the card's GIF button sends: a DM to this person.
let gifTarget = null;

function openMemberCard(member, anchor) {
  const server = state.currentServer || { roles: [] };
  closeFloating();
  const card = $("member-card");
  paintBanner($("mc-banner"), member.role_banner, member.color ? roleGradient(member.color) : bannerColor(member.id));
  const av = $("mc-avatar");
  av.replaceChildren();
  paintAvatar(av, member.id, member.name);
  av.appendChild(el("span", "presence-dot " + (member.presence || "offline")));
  const name = $("mc-name");
  name.textContent = member.name;
  name.style.color = member.color ? `#${member.color}` : "";
  name.parentElement.querySelector(".role-tag")?.remove();
  if (member.tag) name.after(roleTagChip(member.tag, member.tag_color));
  const status = $("mc-status");
  status.replaceChildren();
  const line = statusLine(member);
  const cover = member.activity && igdbCover(member.activity_cover, "t_thumb");
  if (cover) {
    const img = el("img", "activity-cover");
    img.src = cover;
    status.appendChild(img);
  }
  status.appendChild(document.createTextNode(line || PRESENCE_LABELS[member.presence] || ""));
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
  $("mc-about").textContent = member.about || "";
  const actions = $("mc-actions");
  actions.replaceChildren();
  const action = (icon, label, onClick, cls = "") => {
    const btn = el("button", "mc-action " + cls);
    btn.appendChild(svgIcon(icon, 16));
    btn.appendChild(el("span", null, label));
    btn.onclick = e => onClick(e);
    actions.appendChild(btn);
    return btn;
  };
  if (!isMe) {
    actions.appendChild(friendButton(member, () => openMemberCard(member, anchor)));
    action("serverAdd", "Invite", e => openInviteMenu(member, e.currentTarget));
  }
  action("person", "Full Profile", () => openFullProfile(member));
  if (!isMe) {
    action("block", isBlocked(member.id) ? "Unblock" : "Block", () => {
      closeFloating();
      if (isBlocked(member.id)) setBlocked(member.id, member.name, false);
      else openBlock(member.id, member.name);
    });
    const targets = state.currentServer && reportableIn(state.currentServer, member.id)
      ? [state.currentServer]
      : reportServersFor(member.id).filter(s => reportableIn(s, member.id));
    if (targets.length) {
      action("flag", "Report", () => {
        closeFloating();
        openReport({ serverId: targets[0].id, servers: targets, userId: member.id, name: member.name });
      }, "danger");
    }
  }
  $("mc-composer").classList.toggle("hidden", isMe || isBlocked(member.id));
  const input = $("mc-input");
  input.value = "";
  input.placeholder = `Message @${member.name}`;
  input.dataset.user = member.id;
  input.dataset.name = member.name;
  card.classList.remove("hidden");
  // Sit to the left of the member list.
  const a = anchor.getBoundingClientRect();
  const r = card.getBoundingClientRect();
  card.style.left = `${Math.max(8, a.left - r.width - 12)}px`;
  card.style.top = `${Math.max(8, Math.min(a.top, window.innerHeight - r.height - 8))}px`;
}

// Name color and role tag for a message author in the current server.
function decorateAuthor(node, authorId) {
  const member = state.view === "server" && state.currentServer && state.currentServer.members.find(m => m.id === authorId);
  if (!member) return;
  if (member.color) node.style.color = `#${member.color}`;
  if (member.tag) node.after(roleTagChip(member.tag, member.tag_color));
}

function openChooseRoles() {
  const server = state.currentServer;
  if (!server) return;
  closeFloating();
  renderRolePicker(server, $("role-pick-list"));
  showModal("modal-choose-roles");
}

// Fills a list with the server's self-assignable roles as toggles.
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
    if (role.color) name.style.color = `#${role.color}`;
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

// Inline editor under a role row in Server Settings, Roles.
function roleEditor(server, role) {
  const box = el("div", "role-editor");
  const row1 = el("div", "inline-row");
  const color = el("input");
  color.type = "color";
  color.value = role.color ? `#${role.color}` : "#99aab5";
  color.title = "Role color";
  const name = el("input");
  name.type = "text";
  name.maxLength = 50;
  name.value = role.name;
  name.placeholder = "Role name";
  let tagValue = role.tag || null;
  const tagBtn = el("button", "role-tag-pick");
  tagBtn.type = "button";
  const paintTag = () => {
    tagBtn.replaceChildren();
    if (tagValue) {
      tagBtn.appendChild(roleTagChip(tagValue, role.color));
      tagBtn.title = "Change emoji";
    } else {
      tagBtn.appendChild(svgIcon("smilePlus", 18));
      tagBtn.appendChild(el("span", null, "Emoji"));
      tagBtn.title = "Pick an emoji shown next to members' names";
    }
    clearTag.classList.toggle("hidden", !tagValue);
  };
  const clearTag = el("button", "icon-btn role-tag-clear");
  clearTag.type = "button";
  clearTag.title = "Remove emoji";
  clearTag.appendChild(svgIcon("close", 14));
  clearTag.onclick = () => {
    tagValue = null;
    paintTag();
  };
  tagBtn.onclick = e => openEmojiPop(e.currentTarget, emoji => {
    tagValue = emoji;
    paintTag();
  }, { custom: true });
  paintTag();
  row1.append(color, name, tagBtn, clearTag);
  box.appendChild(row1);
  const bannerRow = el("div", "role-banner-row");
  const bannerPreview = el("div", "role-banner-preview");
  paintBanner(bannerPreview, role.banner, role.color ? `#${role.color}` : "var(--bg-active)");
  if (!role.banner) bannerPreview.appendChild(el("span", null, "No banner"));
  const bannerActions = el("div", "role-banner-actions");
  bannerActions.appendChild(el("div", "setting-sub", "Optional banner shown on profile cards of members whose highest role has one, and on the Choose Roles card."));
  const upload = el("button", "btn-secondary btn-small", role.banner ? "Change Banner" : "Upload Banner");
  upload.type = "button";
  upload.onclick = async () => {
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
  };
  bannerActions.appendChild(upload);
  if (role.banner) {
    const remove = el("button", "btn-link danger", "Remove");
    remove.type = "button";
    remove.onclick = async () => {
      try {
        await invoke("set_role_banner", { serverId: server.id, roleId: role.id, data: null });
        await refreshCurrentServer();
        openServerSettings();
      } catch (err) {
        toast(String(err));
      }
    };
    bannerActions.appendChild(remove);
  }
  bannerRow.append(bannerPreview, bannerActions);
  box.appendChild(bannerRow);
  const pick = el("label", "role-self");
  const check = el("input");
  check.type = "checkbox";
  check.checked = role.self_assignable;
  pick.append(check, el("span", null, "Members can pick this role themselves (Choose Roles)"));
  box.appendChild(pick);
  const actions = el("div", "form-actions");
  const save = el("button", "btn-primary btn-small", "Save Role");
  save.onclick = async () => {
    try {
      await invoke("update_role", {
        serverId: server.id,
        roleId: role.id,
        name: name.value,
        color: color.value.replace("#", ""),
        selfAssignable: check.checked,
        tag: tagValue,
      });
      await refreshCurrentServer();
      openServerSettings();
      toast("Role saved");
    } catch (e) {
      toast(String(e));
    }
  };
  actions.appendChild(save);
  box.appendChild(actions);
  return box;
}

// ---------- Safety: blocking, reports, message requests ----------

let safety = { blocked: [], dm_policy: "everyone", spam_filter: true };
// Blocked messages the user chose to reveal this session.
const revealedBlocked = new Set();

async function loadSafety() {
  try {
    safety = await invoke("get_safety");
  } catch {
    /* keep the last known settings */
  }
}

function isBlocked(userId) {
  return Boolean(userId) && safety.blocked.some(b => b.id === userId);
}

function renderSafetySettings() {
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

// Re-renders whatever shows people or messages after a block change.
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

// One collapsed row for a run of messages from people you blocked.
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

// Right-click menu on a member.
let menuMember = null;

// Servers we share with someone where a report could go to the owner.
function reportServersFor(userId) {
  const person = state.people.get(userId);
  return (person ? person.servers : [])
    .map(s => state.servers.find(x => x.id === s.id) || s)
    .filter(s => s && s.id);
}

// Right-click menu for a person: in the member list, a voice channel or
// a DM. Built fresh each time so it reflects permissions and settings.
function openMemberMenu(member, x, y, { dm = false } = {}) {
  if (state.user && member.id === state.user.id) return;
  closeFloating();
  menuMember = member;
  const server = dm ? null : state.currentServer;
  const inServer = Boolean(server && server.members.some(m => m.id === member.id));
  const menu = $("member-menu");
  menu.replaceChildren(el("div", "ctx-title", member.name));
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

  item("Profile", () => openMemberCard(fullMember(member), { getBoundingClientRect: () => ({ left: x, top: y }) }));
  if (!(dm && state.currentDm && state.currentDm.user === member.id)) {
    item("Message", () => messageUser(member.id, member.name));
  }
  sep();

  // Personal voice controls, on this device only.
  const voice = state.voice || {};
  const mutedForMe = (voice.muted_users || []).includes(member.id);
  const mute = item(mutedForMe ? "Unmute" : "Mute", () => setUserVoice(member.id, { muted: !mutedForMe }), "ctx-check" + (mutedForMe ? " checked" : ""));
  mute.title = "Only you stop hearing them";
  const volume = Math.round(((voice.user_volumes || {})[member.id] ?? 1) * 100);
  const slider = el("div", "ctx-slider");
  const label = el("div", "ctx-slider-label");
  label.append(el("span", null, "User Volume"), el("span", null, `${volume}%`));
  const range = el("input");
  range.type = "range";
  range.min = "0";
  range.max = "200";
  range.value = String(volume);
  range.oninput = () => {
    label.lastChild.textContent = `${range.value}%`;
    syncRangeFill(range);
  };
  range.onchange = () => setUserVoice(member.id, { volume: Number(range.value) / 100 });
  slider.append(label, range);
  menu.appendChild(slider);
  syncRangeFill(range);
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

  // Moderation, for people with the permissions.
  if (inServer && member.id !== server.owner && (server.can_mute || server.can_kick || server.can_ban)) {
    sep();
    if (server.can_mute) {
      const sm = (server.server_muted || []).includes(member.id);
      const sd = (server.server_deafened || []).includes(member.id);
      item("Server Mute", () => moderateVoice(member, !sm, sd), "ctx-check" + (sm ? " checked" : ""));
      item("Server Deafen", () => moderateVoice(member, sm, !sd), "ctx-check" + (sd ? " checked" : ""));
    }
    if (server.can_kick) {
      item(`Kick ${member.name}`, async () => {
        if (!confirm(`Kick ${member.name} from ${server.name}?`)) return;
        try {
          await invoke("kick_member", { serverId: server.id, userId: member.id });
          await refreshCurrentServer();
          toast(`Kicked ${member.name}`);
        } catch (e) {
          toast(String(e));
        }
      }, "danger");
    }
    if (server.can_ban) {
      item(`Ban ${member.name}`, async () => {
        if (!confirm(`Ban ${member.name} from ${server.name}? They are removed and cannot rejoin until unbanned.`)) return;
        try {
          await invoke("ban_member", { serverId: server.id, userId: member.id });
          await refreshCurrentServer();
          toast(`Banned ${member.name}`);
        } catch (e) {
          toast(String(e));
        }
      }, "danger");
    }
  }
  placeMenuAt(menu, x, y);
}

// The richest view of a person we have: the server member if present.
function fullMember(person) {
  const fromServer = state.currentServer && state.currentServer.members.find(m => m.id === person.id);
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

async function moderateVoice(member, muted, deafened) {
  try {
    await invoke("moderate_voice", { serverId: state.currentServer.id, userId: member.id, muted, deafened });
    await refreshCurrentServer();
  } catch (e) {
    toast(String(e));
  }
}

// Reports go to the owner, so owners have nobody to report to.
function reportableIn(server, userId) {
  return Boolean(server && state.user && server.owner !== state.user.id && userId !== server.owner && userId !== state.user.id);
}

let reportTarget = null;

function openReport(target) {
  reportTarget = target;
  const servers = target.servers || [];
  $("report-server-row").classList.toggle("hidden", servers.length < 2);
  const pick = $("report-server");
  pick.replaceChildren(...servers.map(s => {
    const opt = el("option", null, s.name);
    opt.value = s.id;
    return opt;
  }));
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
    preview.appendChild(buildMessageContent(target.excerpt));
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
      quote.appendChild(buildMessageContent(report.excerpt));
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

// Accept, decline or block for a conversation still in Message Requests.
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

// ---------- Timed status ----------

let durationMode = null; // { kind: "presence", presence } or { kind: "status" }

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
  } catch (e) {
    toast(String(e));
  }
  renderProfilePopover();
}

function openDurations(mode, anchor) {
  durationMode = mode;
  $("pp-durations-title").textContent = mode.kind === "presence"
    ? `${({ invisible: "Invisible", ...PRESENCE_LABELS })[mode.presence] || "Status"} for how long?`
    : "Clear custom status after";
  const pad = n => String(n).padStart(2, "0");
  const later = new Date(Date.now() + 2 * 3600e3);
  $("pp-custom-time").value = `${later.getFullYear()}-${pad(later.getMonth() + 1)}-${pad(later.getDate())}T${pad(later.getHours())}:${pad(later.getMinutes())}`;
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
    btn.querySelector(".pp-until")?.remove();
    const active = btn.dataset.presence === (user.presence || "online");
    if (active && v.presence_until && btn.dataset.presence !== "online") {
      btn.querySelector("span:last-child").appendChild(el("small", "pp-until", `Until ${formatUntil(v.presence_until)}`));
    }
  }
  const hasStatus = Boolean(user.custom_status || user.status_emoji);
  const expiry = $("pp-status-expiry");
  expiry.classList.toggle("hidden", !hasStatus);
  expiry.textContent = v.status_until ? `Clears at ${formatUntil(v.status_until)}` : "Clear after: Never";
}

// ---------- Custom emojis ----------

// "<:name:blob@provider>" carries the image, so it renders anywhere.
const EMOJI_TOKEN_RE = /<:([a-z0-9_]{2,32}):([A-Za-z0-9]{1,128}@[A-Za-z0-9]{1,128})>/g;
const MAX_EMOJIS = 50;
const MAX_SOUNDS = 24;
const MAX_SOUND_SECONDS = 15;
const expressionUrls = new Map(); // ref -> Promise<data URL or null>

function expressionUrl(ref) {
  if (!expressionUrls.has(ref)) {
    expressionUrls.set(ref, invoke("fetch_expression", { blob: ref }).catch(() => {
      // Let a later render retry, e.g. once the uploader is back online.
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

// Tokens back to ":name:", for editing, previews and notifications.
function decodeCustomEmojis(text) {
  return String(text || "").replace(EMOJI_TOKEN_RE, ":$1:");
}

function plainText(text) {
  // Server excerpts can cut a token in half; drop the stub.
  return decodeCustomEmojis(text).replace(/<:[^>\s]*$/, "");
}

function isEmojiOnly(text) {
  const tokens = text.match(EMOJI_TOKEN_RE) || [];
  return tokens.length > 0 && tokens.length <= 27 && !text.replace(EMOJI_TOKEN_RE, "").trim();
}

let shareableEmojis = [];

async function loadShareableEmojis() {
  try {
    shareableEmojis = await invoke("list_shareable_emojis");
  } catch {
    shareableEmojis = [];
  }
}

// Custom emojis usable where you are typing: this server's own, then
// shareable ones from your other servers. DMs only get shareable ones.
function usableCustomEmojis() {
  const here = state.view === "server" ? state.currentServer : null;
  const local = here && here.emojis && here.emojis.length ? { id: here.id, name: here.name, emojis: here.emojis } : null;
  const others = new Map();
  for (const e of shareableEmojis) {
    if (here && e.server === here.id) continue;
    if (!others.has(e.server)) others.set(e.server, { id: e.server, name: e.server_name, emojis: [] });
    others.get(e.server).emojis.push({ name: e.name, image: e.image });
  }
  return { local, external: [...others.values()] };
}

function findCustomEmoji(name) {
  const { local, external } = usableCustomEmojis();
  return (local && local.emojis.find(e => e.name === name))
    || external.flatMap(g => g.emojis).find(e => e.name === name)
    || null;
}

// ":name:" typed or picked in the composer becomes a token on send.
function encodeCustomEmojis(text) {
  return text.replace(/(?<![<\w]):([a-z0-9_]{2,32}):/gi, (whole, name) => {
    const e = findCustomEmoji(name.toLowerCase());
    return e ? `<:${e.name}:${e.image}>` : whole;
  });
}

// Message text: custom emoji tokens become images, the rest goes
// through mention highlighting.
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

// Custom emoji sections for a picker grid. `onPick` gets the emoji.
function appendCustomEmojiSections(grid, query, onPick) {
  const { local, external } = usableCustomEmojis();
  const groups = [local, ...external].filter(Boolean);
  let any = false;
  for (const group of groups) {
    const matches = group.emojis.filter(e => !query || e.name.includes(query));
    if (!matches.length) continue;
    any = true;
    const head = el("div", "emoji-section");
    head.appendChild(el("span", null, group.name));
    if (group !== local) head.appendChild(el("span", "emoji-section-tag", "Other Server"));
    grid.appendChild(head);
    for (const e of matches) {
      const btn = el("button", "custom");
      btn.title = `:${e.name}:`;
      btn.appendChild(customEmojiImg(e.name, e.image));
      btn.onclick = () => onPick(e);
      grid.appendChild(btn);
    }
  }
  if (any) grid.appendChild(el("div", "emoji-section", "Standard"));
}

// Emoji uploads: stills and big GIFs become a 128px PNG; small GIFs are
// kept as uploaded so they stay animated.
async function prepareEmojiImage(src) {
  const blob = await (await fetch(src)).blob();
  if (blob.type === "image/gif" && blob.size <= 256 * 1024) {
    return { data: await blobToBase64(blob), url: src, flattened: false };
  }
  const img = await new Promise((resolve, reject) => {
    const i = new Image();
    i.onload = () => resolve(i);
    i.onerror = () => reject(new Error("could not read that image"));
    i.src = src;
  });
  const scale = Math.min(1, 128 / Math.max(img.naturalWidth, img.naturalHeight));
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

// A pending upload row: preview, editable name, Save and Cancel.
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

function renderEmojiSettings(server) {
  const emojis = server.emojis || [];
  $("emoji-share-toggle").checked = Boolean(server.emojis_shareable);
  $("emoji-share-toggle").disabled = !server.can_manage_server;
  $("emoji-share-row").classList.toggle("readonly", !server.can_manage_server);
  $("emoji-upload").classList.toggle("hidden", !server.can_manage_expressions);
  $("emoji-upload").disabled = emojis.length >= MAX_EMOJIS;
  $("emoji-count").textContent = `${emojis.length} of ${MAX_EMOJIS} slots used`;
  const list = $("emoji-list");
  list.replaceChildren();
  for (const e of emojis) {
    const row = el("div", "expr-row");
    row.appendChild(customEmojiImg(e.name, e.image, "custom-emoji expr-preview"));
    row.appendChild(el("span", "expr-name", `:${e.name}:`));
    if (server.can_manage_expressions) {
      const del = el("button", "icon-btn expr-delete");
      del.title = `Delete :${e.name}:`;
      del.appendChild(svgIcon("trash", 18));
      del.onclick = async () => {
        try {
          await invoke("remove_emoji", { serverId: server.id, name: e.name });
          await refreshExpressions();
        } catch (err) {
          toast(String(err));
        }
      };
      row.appendChild(del);
    }
    list.appendChild(row);
  }
  if (!emojis.length) list.appendChild(el("div", "manage-empty", "No custom emojis yet"));
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

// ---------- Soundboard ----------

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
  try { localStorage.setItem(SOUNDBOARD_KEY, JSON.stringify(prefs)); } catch { /* session only */ }
}

// Clips bundled with the app, in every server. Ids match DEFAULT_SOUNDS
// in expressions.rs; peers play them by name ("default:<id>").
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
  // A few overlapping clips are fun; a wall of them is not.
  if (soundboardPlaying.size >= 3) {
    const oldest = soundboardPlaying.values().next().value;
    oldest.pause();
    soundboardPlaying.delete(oldest);
  }
  const audio = new Audio(url);
  audio.volume = Math.max(0, Math.min(1, volume));
  soundboardPlaying.add(audio);
  const done = () => soundboardPlaying.delete(audio);
  audio.addEventListener("ended", done);
  // Clips are capped at upload, but never let one run on.
  setTimeout(() => {
    audio.pause();
    done();
  }, (MAX_SOUND_SECONDS + 1) * 1000);
  audio.play().catch(done);
}

// Someone in our voice channel (maybe us) played a clip.
function onSoundboardPlay(play) {
  const prefs = soundboardPrefs();
  flashSoundboardUser(play.user, defaultSound(play.name)?.label || play.name);
  if (!prefs.enabled || state.voice.deafened) return;
  playExpressionSound(play.blob, (play.volume ?? 1) * prefs.volume);
}

// Shows the clip name next to whoever played it in the voice list.
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

// Reads a clip's length so over-long sounds are refused before upload.
async function audioDuration(dataUrl) {
  const buffer = await (await fetch(dataUrl)).arrayBuffer();
  const ctx = new AudioContext();
  try {
    const decoded = await ctx.decodeAudioData(buffer);
    return decoded.duration;
  } finally {
    ctx.close();
  }
}

function pickSoundFile() {
  if (IS_TAURI) return invoke("pick_sound");
  return new Promise(resolve => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "audio/mpeg,audio/ogg,audio/wav,.mp3,.ogg,.wav";
    input.onchange = () => {
      const file = input.files && input.files[0];
      if (!file) return resolve(null);
      if (file.size > 1024 * 1024) {
        toast("Sounds must be under 1 MB");
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

async function uploadSound() {
  const server = state.currentServer;
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
  let seconds;
  try {
    seconds = await audioDuration(dataUrl);
  } catch {
    toast("Could not read that sound file");
    return;
  }
  if (seconds > MAX_SOUND_SECONDS + 0.2) {
    toast(`That clip is ${seconds.toFixed(1)} seconds; sounds can be up to ${MAX_SOUND_SECONDS}`);
    return;
  }
  const play = el("button", "icon-btn sound-preview");
  play.title = "Preview";
  play.appendChild(svgIcon("play", 18));
  play.onclick = () => {
    const audio = new Audio(dataUrl);
    audio.volume = soundboardPrefs().volume;
    audio.play().catch(() => {});
  };
  const taken = (server.sounds || []).map(s => s.name);
  const row = pendingExpressionRow(play, freeExpressionName(expressionName(stem, "sound"), taken), async clean => {
    await invoke("add_sound", { serverId: server.id, name: clean, data: dataUrl.split(",")[1], volume: 1 });
    await refreshExpressions();
    toast(`Added ${clean} to the soundboard`);
  });
  $("sound-list").querySelector(".manage-empty")?.remove();
  $("sound-list").prepend(row);
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
  $("sb-server").textContent = "";
  const addButton = (name, label) => {
    const btn = el("button", "sb-sound");
    btn.appendChild(svgIcon("soundboard", 14));
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
  grid.appendChild(el("div", "sb-section", "Default"));
  for (const sound of DEFAULT_SOUNDS) addButton(`default:${sound.id}`, sound.label);
  grid.appendChild(el("div", "sb-section", server.name));
  for (const sound of server.sounds || []) addButton(sound.name, sound.name);
  if (!(server.sounds || []).length) {
    grid.appendChild(el("div", "sb-empty", server.can_upload_sounds
      ? "No server sounds yet. Add some in Server Settings under Soundboard."
      : "This server has no sounds of its own yet."));
  }
  $("sb-muted").classList.toggle("hidden", soundboardPrefs().enabled);
  pop.classList.remove("hidden");
  placePopup(pop, $("vp-soundboard"), "above");
}

// ---------- Mentions ----------

// People who can be mentioned where you are: server members plus
// @everyone and @here, or in a DM the two of you.
function mentionCandidates() {
  if (state.view === "dm" && state.currentDm) {
    const me = state.user ? [{ id: state.user.id, name: state.user.display_name || state.user.username }] : [];
    return [{ id: state.currentDm.user, name: state.currentDm.name }, ...me];
  }
  if (!state.currentServer) return [];
  const members = state.currentServer.members.map(m => ({ id: m.id, name: m.name, presence: m.presence }));
  return [
    ...members,
    { id: "everyone", name: "everyone", special: "Notify everyone in this server" },
    { id: "here", name: "here", special: "Notify everyone online" },
  ];
}

let mentionRegexCache = { key: "", re: null };

// Matches "@name" for any known name, longest names first so "@Commy C"
// wins over "@Commy".
function mentionRegex() {
  const names = mentionCandidates().map(c => c.name).filter(Boolean);
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

function appendTextWithMentions(div, text) {
  const re = mentionRegex();
  if (!re) {
    div.appendChild(document.createTextNode(text));
    return;
  }
  const myNames = state.user ? [state.user.username, state.user.display_name].filter(Boolean).map(n => n.toLowerCase()) : [];
  re.lastIndex = 0;
  let last = 0;
  let m;
  while ((m = re.exec(text))) {
    if (m.index > last) div.appendChild(document.createTextNode(text.slice(last, m.index)));
    const name = m[1].toLowerCase();
    const forMe = myNames.includes(name) || name === "everyone" || name === "here";
    div.appendChild(el("span", "mention" + (forMe ? " me" : ""), m[0]));
    last = m.index + m[0].length;
  }
  if (last < text.length) div.appendChild(document.createTextNode(text.slice(last)));
}

// Live @ autocomplete for a text input.
const mentionPop = { input: null, start: 0, items: [], index: 0 };

function mentionQueryAt(input) {
  const caret = input.selectionStart ?? input.value.length;
  const before = input.value.slice(0, caret);
  const m = before.match(/(^|\s)@([^\s@]{0,32})$/);
  return m ? { start: caret - m[2].length - 1, query: m[2].toLowerCase() } : null;
}

function updateMentionPop(input) {
  const found = mentionQueryAt(input);
  const pop = $("mention-pop");
  if (!found) {
    closeMentionPop();
    return;
  }
  const items = mentionCandidates()
    .filter(c => c.name && c.name.toLowerCase().includes(found.query))
    .sort((a, b) => {
      const ap = a.name.toLowerCase().startsWith(found.query) ? 0 : 1;
      const bp = b.name.toLowerCase().startsWith(found.query) ? 0 : 1;
      return ap - bp || (a.special ? 1 : 0) - (b.special ? 1 : 0) || a.name.localeCompare(b.name);
    })
    .slice(0, 8);
  if (!items.length) {
    closeMentionPop();
    return;
  }
  Object.assign(mentionPop, { input, start: found.start, items, index: Math.min(mentionPop.index, items.length - 1) });
  if (pop.classList.contains("hidden")) mentionPop.index = 0;
  pop.replaceChildren(el("div", "mention-head", found.query ? `Members matching @${found.query}` : "Members"));
  items.forEach((item, i) => {
    const row = el("button", "mention-row" + (i === mentionPop.index ? " active" : ""));
    if (item.special) {
      row.appendChild(el("span", "mention-at", "@"));
      row.appendChild(el("span", "mention-name", `@${item.name}`));
      row.appendChild(el("span", "mention-hint", item.special));
    } else {
      row.appendChild(makeAvatar(item.id, item.name, "avatar avatar-sm"));
      row.appendChild(el("span", "mention-name", item.name));
    }
    row.onmousedown = e => {
      e.preventDefault();
      pickMention(i);
    };
    pop.appendChild(row);
  });
  // Sit just above whichever composer is typing.
  const r = input.closest(".composer").getBoundingClientRect();
  pop.style.left = `${r.left}px`;
  pop.style.width = `${Math.min(r.width, 420)}px`;
  pop.style.bottom = `${window.innerHeight - r.top + 6}px`;
  pop.classList.remove("hidden");
}

function closeMentionPop() {
  $("mention-pop").classList.add("hidden");
  mentionPop.items = [];
}

function pickMention(i) {
  const item = mentionPop.items[i];
  const input = mentionPop.input;
  if (!item || !input) return;
  const caret = input.selectionStart ?? input.value.length;
  const insert = `@${item.name} `;
  input.value = input.value.slice(0, mentionPop.start) + insert + input.value.slice(caret);
  const pos = mentionPop.start + insert.length;
  input.setSelectionRange(pos, pos);
  input.focus();
  closeMentionPop();
}

function mentionTypedInFull(input) {
  const found = mentionQueryAt(input);
  const item = mentionPop.items[mentionPop.index];
  return !!(found && item && item.name.toLowerCase() === found.query);
}

function wireMentions(input) {
  input.addEventListener("input", () => updateMentionPop(input));
  input.addEventListener("click", () => updateMentionPop(input));
  input.addEventListener("blur", () => setTimeout(closeMentionPop, 100));
  // Capture phase so Enter picks a name instead of sending.
  input.addEventListener("keydown", e => {
    if ($("mention-pop").classList.contains("hidden") || mentionPop.input !== input) return;
    const n = mentionPop.items.length;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      mentionPop.index = (mentionPop.index + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      updateMentionPop(input);
    } else if (e.key === "Enter" && mentionTypedInFull(input)) {
      // "@Name" already typed out: Enter sends instead of picking again.
      closeMentionPop();
      return;
    } else if (e.key === "Enter" || e.key === "Tab") {
      pickMention(mentionPop.index);
    } else if (e.key === "Escape") {
      closeMentionPop();
    } else {
      return;
    }
    e.preventDefault();
    e.stopImmediatePropagation();
  }, true);
}

function buildMessageContent(text) {
  const div = el("div", "msg-content");
  // A message that is only an invite link shows the Join card, not the raw link.
  if (looksLikeInviteLink(text)) {
    div.appendChild(inviteCard(text));
    return div;
  }
  const urlRe = /https?:\/\/[^\s<>"']+/g;
  let last = 0;
  let match;
  while ((match = urlRe.exec(text))) {
    if (match.index > last) {
      appendRichText(div, text.slice(last, match.index));
    }
    const url = match[0];
    const a = el("a", null, url);
    a.href = "#";
    a.title = `Open ${url}`;
    a.onclick = e => {
      e.preventDefault();
      invoke("open_url", { url }).catch(err => toast(String(err)));
    };
    div.appendChild(a);
    last = match.index + url.length;
  }
  if (last < text.length) {
    appendRichText(div, text.slice(last));
  }
  if (isEmojiOnly(text)) div.classList.add("jumbo");
  const invite = inviteCard(text);
  if (invite) div.appendChild(invite);
  return div;
}

function extractImageUrls(text) {
  const matches = text.match(/https?:\/\/[^\s<>"']+\.(?:gif|png|jpe?g|webp)(?:\?[^\s<>"']*)?/gi);
  if (matches) return matches.slice(0, 4);
  // A message that is nothing but one link is worth trying as an
  // image even without a file extension; onerror removes the miss.
  const lone = text.trim();
  if (findInviteLink(lone)) return [];
  return /^https?:\/\/\S+$/i.test(lone) ? [lone] : [];
}

// Source URL of an image copied from a web page, if the clipboard
// html flavor carries one. DOMParser documents are inert: nothing
// in the parsed markup loads or runs.
function imageSourceFromHtml(html) {
  if (!html) return null;
  const doc = new DOMParser().parseFromString(html, "text/html");
  const src = doc.querySelector("img")?.getAttribute("src") || "";
  return /^https?:\/\//i.test(src) ? src : null;
}

// Images finish loading after the chat has already scrolled to the
// bottom; re-pin it unless the user has scrolled up to read history.
function keepScrolledOnLoad(img) {
  img.addEventListener("load", () => {
    const box = $("messages");
    const slack = img.offsetHeight + 120;
    if (box.scrollHeight - box.scrollTop - box.clientHeight <= slack) {
      box.scrollTop = box.scrollHeight;
    }
  });
}

// An image embedded from a link someone pasted, as opposed to a file
// they uploaded: it carries a visible source URL above the image.
function renderImageEmbed(url) {
  const wrap = el("div", "image-embed");
  const source = el("a", "embed-source", embedSourceLabel(url));
  source.href = "#";
  source.title = `Open ${url}`;
  source.onclick = e => {
    e.preventDefault();
    invoke("open_url", { url }).catch(err => toast(String(err)));
  };
  const img = el("img", "chat-image");
  keepScrolledOnLoad(img);
  img.src = viaAccountServer(url);
  img.loading = "lazy";
  img.onclick = () => openLightbox(img.src, url, null);
  img.onerror = () => wrap.remove();
  wrap.appendChild(source);
  wrap.appendChild(img);
  return wrap;
}

function embedSourceLabel(url) {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

// Full-size overlay; for link embeds the source URL under the image
// opens in the system browser, uploads just show the filename.
function openLightbox(src, sourceUrl, filename) {
  $("lightbox-img").src = src;
  const source = $("lightbox-source");
  source.textContent = sourceUrl || filename || "";
  source.classList.toggle("hidden", !(sourceUrl || filename));
  source.classList.toggle("linked", Boolean(sourceUrl));
  source.onclick = e => {
    e.preventDefault();
    e.stopPropagation();
    if (sourceUrl) invoke("open_url", { url: sourceUrl }).catch(err => toast(String(err)));
  };
  $("lightbox").classList.remove("hidden");
}

function closeLightbox() {
  $("lightbox").classList.add("hidden");
  $("lightbox-img").src = "";
}

const imagePreviewCache = new Map();

function renderInlineImage(att) {
  const img = el("img", "chat-image");
  keepScrolledOnLoad(img);
  img.alt = att.filename;
  img.onclick = () => {
    if (img.src) openLightbox(img.src, null, att.filename);
  };
  const cached = imagePreviewCache.get(att.blob);
  if (cached) {
    img.src = cached;
  } else {
    invoke("get_attachment_preview", {
      blob: att.blob,
      provider: att.provider ?? null,
      mime: att.mime,
    })
      .then(dataUrl => {
        imagePreviewCache.set(att.blob, dataUrl);
        img.src = dataUrl;
      })
      .catch(() => img.remove());
  }
  return img;
}

function renderShareButton() {
  const btn = $("share-btn");
  const inVoice = Boolean(state.voiceChannelId);
  btn.classList.toggle("hidden", !inVoice);
  btn.classList.toggle("active", state.sharing);
  btn.title = state.sharing ? "Stop Sharing" : "Share Your Screen";
  renderVoicePanel();
}

// Voice connection card above the user panel, with screen share controls.
function renderVoicePanel() {
  const panel = $("voice-panel");
  const vs = state.voiceServer;
  panel.classList.toggle("hidden", !(state.voiceChannelId && vs));
  if (!state.voiceChannelId || !vs) return;

  $("vp-where").textContent = `${vs.channelName} / ${vs.name}`;
  // One click to stream the game you are playing.
  const playing = !state.sharing && state.user && state.user.activity && state.user.activity.startsWith("Playing ")
    ? state.user.activity.slice(8)
    : null;
  const gameBtn = $("vp-game-share");
  gameBtn.classList.toggle("hidden", !playing);
  if (playing) {
    gameBtn.replaceChildren(svgIcon("screen", 16), el("span", null, `Share ${playing}`));
    gameBtn.title = `Stream ${playing} to your voice channel at 60 fps`;
  }
  $("vp-signal").replaceChildren(svgIcon("signal", 14));
  setIcon($("vp-disconnect"), "hangup", 20);
  setIcon($("vp-nc"), "noise", 18);
  $("vp-nc").classList.toggle("on", state.voice.noise_cancellation);
  $("vp-nc").title = `Noise Cancellation: ${state.voice.noise_cancellation ? "on" : "off"}`;

  $("vp-share-icon").replaceChildren(svgIcon(state.sharing ? "screenOff" : "screen", 18));
  $("vp-camera-icon").replaceChildren(svgIcon(state.cameraOn ? "camera" : "cameraOff", 18));
  $("vp-camera-label").textContent = state.cameraOn ? "Camera On" : "Camera";
  $("vp-camera").classList.toggle("active", Boolean(state.cameraOn));
  $("vp-camera").title = state.cameraOn ? "Turn Off Camera" : "Turn On Camera";
  renderCamGrid();
  $("vp-share-label").textContent = state.sharing ? "Stop Sharing" : "Share Screen";
  $("vp-share").title = state.sharing ? "Stop Sharing" : "Share Your Screen";
  $("vp-share").classList.toggle("active", state.sharing);

  $("vp-live").classList.toggle("hidden", !state.sharing);
  $("vp-live-source").textContent = state.shareSource ? `Streaming ${state.shareSource}` : "";
  $("vp-live-title").textContent = `Live in ${vs.channelName}`;

  $("vp-watch").classList.toggle("hidden", !state.watchingUser);
  $("vp-watch-label").textContent = state.watchingUser ? `Watching ${state.watchingName || "a stream"}` : "";

  // Other people streaming in our voice server.
  const streams = $("vp-streams");
  streams.innerHTML = "";
  const server = state.currentServer && state.currentServer.id === vs.id ? state.currentServer : null;
  const others = server
    ? (server.screen_shares || []).filter(s => (!state.user || s.user !== state.user.id) && s.user !== state.watchingUser)
    : [];
  for (const share of others) {
    const row = el("div", "vp-stream");
    row.appendChild(el("span", "live-badge", "LIVE"));
    row.appendChild(el("span", "vp-stream-name", share.name));
    const watch = el("button", "vp-pill", "Watch");
    watch.onclick = () => watchScreen(share.user, share.name);
    row.appendChild(watch);
    streams.appendChild(row);
  }
}

async function disconnectVoice() {
  if (!state.voiceServer) return;
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
  const bits = [];
  if (state.voice.muted) bits.push("muted");
  if (state.voice.deafened) bits.push("deafened");
  const label = presence === "idle" ? "Idle" : PRESENCE_LABELS[presence] || "Invisible";
  $("me-status").textContent = bits.length ? `${label}, ${bits.join(", ")}` : (statusLine(state.user) || label);
  $("mute-btn").classList.toggle("danger", state.voice.muted);
  $("deafen-btn").classList.toggle("danger", state.voice.deafened);
  setIcon($("mute-btn"), state.voice.muted ? "micOff" : "mic");
  setIcon($("deafen-btn"), state.voice.deafened ? "headsetOff" : "headset");
  setIcon($("settings-btn"), "gear");
  $("mute-btn").title = state.voice.muted ? "Unmute" : "Mute";
  $("deafen-btn").title = state.voice.deafened ? "Undeafen" : "Deafen";
  // Our mute state also shows next to our name in the voice channel.
  if (state.currentServer && state.voiceChannelId && state.view === "server") renderChannels();
}

// ---------- Actions ----------

async function refreshServers() {
  state.servers = await invoke("list_servers");
  loadShareableEmojis();
  renderServerRail();
}

// Switches the three-pane layout between server mode and DM mode.
function setViewMode(mode) {
  state.view = mode;
  const dm = mode === "dm";
  $("member-list").classList.toggle("hidden", dm);
  $("invite-btn").classList.toggle("hidden", dm);
  $("server-settings-btn").classList.toggle("hidden", dm);
  $("server-name").classList.toggle("hidden", dm);
  if (dm) paintSidebarBanner();
  $("dm-search").classList.toggle("hidden", !dm);
  $("attach-btn").classList.toggle("hidden", dm);
  $("home-btn").classList.toggle("active", dm);
  $("app").classList.toggle("dm-mode", dm);
  if (dm) $("cam-grid").classList.add("hidden");
  if (dm) syncAwaitedBar();
  if (!dm) {
    $("chat-hash").textContent = "#";
    $("chat-presence").classList.add("hidden");
    $("dm-profile").classList.add("hidden");
    $("dm-profile-btn").classList.add("hidden");
    $("dm-search").value = "";
  }
  if (dm) $("share-btn").classList.add("hidden");
  else renderShareButton();
}

async function selectServer(serverId) {
  try {
    state.currentServer = await invoke("get_server", { serverId });
  } catch (e) {
    toast(String(e));
    return;
  }
  setViewMode("server");
  state.currentDm = null;
  const firstText = state.currentServer.channels.find(c => c.kind === "text");
  renderServerRail();
  renderMembers();
  await selectChannel(firstText || null);
}

// ---------- Direct messages ----------

// Everyone we share a server with, keyed by user id. DMs have no presence
// of their own, so presence, status and mutual servers come from here.
async function refreshPeople() {
  const people = new Map();
  for (const summary of state.servers) {
    let server;
    try {
      server = await invoke("get_server", { serverId: summary.id });
    } catch {
      continue;
    }
    rememberAvatars(server.members);
    for (const member of server.members) {
      if (state.user && member.id === state.user.id) continue;
      const person = people.get(member.id) || {
        id: member.id, name: member.name, presence: member.presence,
        custom_status: member.custom_status, status_emoji: member.status_emoji, activity: member.activity, servers: [],
      };
      // Prefer the liveliest presence seen across servers.
      if (person.presence === "offline" && member.presence !== "offline") person.presence = member.presence;
      person.custom_status ||= member.custom_status;
      person.status_emoji ||= member.status_emoji;
      person.activity ||= member.activity;
      person.servers.push({ id: server.id, name: server.name });
      people.set(member.id, person);
    }
  }
  state.people = people;
}

function personPresence(userId) {
  const person = state.people.get(userId);
  if (!person || person.presence === "invisible") return "offline";
  return person.presence;
}

const PRESENCE_LABELS = { online: "Online", idle: "Idle", do_not_disturb: "Do Not Disturb", offline: "Offline" };

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
  } else if (threads.length) {
    await selectDm(threads[0].user, threads[0].name);
  } else {
    renderDmHome();
  }
}

// Shown when there are no conversations yet.
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
  if (!state.servers.length) {
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

// The first thing a new person sees: no servers yet, two ways to get one.
function buildWelcomeHome() {
  const home = el("div", "empty-state welcome-home");
  const mark = el("img", "welcome-mark");
  mark.src = "images/commy_v2.png";
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
  const list = $("channel-list");
  list.innerHTML = "";
  const threads = await invoke("list_dm_threads");
  state.dmThreads = threads;
  const query = $("dm-search").value.trim().toLowerCase();
  const matches = threads.filter(t => !query || t.name.toLowerCase().includes(query));
  const shown = matches.filter(t => t.kind === "inbox");
  const requests = matches.filter(t => t.kind !== "inbox");

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
  if (!query && (friends.friends.length || friends.incoming.length)) {
    list.appendChild(el("div", "channel-group dm-group", `Friends - ${friends.friends.length}`));
    for (const req of friends.incoming) {
      const row = el("div", "channel dm-thread friend-request");
      row.appendChild(avatarWithPresence(req.name, req.id));
      const meta = el("div", "dm-meta");
      meta.appendChild(el("div", "dm-name", req.name));
      meta.appendChild(el("div", "dm-preview", "Wants to be friends"));
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
      meta.appendChild(el("div", "dm-preview", (person && statusLine(person)) || PRESENCE_LABELS[personPresence(friend.id)] || "Offline"));
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
  const rows = [...shown];
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
    const row = el("div", "channel dm-thread");
    if (state.currentDm && thread.user === state.currentDm.user) row.classList.add("active");
    if (unread.dms.get(thread.user)) row.classList.add("unread");
    row.appendChild(avatarWithPresence(thread.name, thread.user));
    const meta = el("div", "dm-meta");
    const top = el("div", "dm-top");
    top.appendChild(el("span", "dm-name", thread.name));
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
    list.appendChild(row);
  }
  if (query) {
    // Offer to start a chat with matching people who have no thread yet.
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

async function selectDm(userId, name) {
  state.currentDm = { user: userId, name };
  clearDmUnread(userId);
  if (state.view !== "dm") setViewMode("dm");
  const presence = personPresence(userId);
  $("chat-hash").textContent = "@";
  $("channel-name").textContent = name;
  $("chat-presence").className = `presence-dot inline ${presence}`;
  $("chat-presence").title = PRESENCE_LABELS[presence] || "Offline";
  const person = state.people.get(userId);
  $("channel-topic").textContent = (person && statusLine(person)) || "";
  $("message-input").placeholder = `Message @${name}`;
  $("dm-profile-btn").classList.remove("hidden");
  renderDmProfile(userId, name);
  const threads = await renderDmThreads();
  renderDmRequestBar(threads.find(t => t.user === userId) || null);
  const messages = await invoke("get_dm_messages", { userId });
  renderMessages(messages.map(dmToRenderable));
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
  $("dm-profile-status").textContent = (person && statusLine(person)) || PRESENCE_LABELS[presence];
  const servers = person ? person.servers : [];
  $("dm-profile-mutual-label").textContent = `Mutual Servers - ${servers.length}`;
  const list = $("dm-profile-servers");
  list.innerHTML = "";
  for (const server of servers) {
    const row = el("button", "dm-profile-server");
    row.appendChild(el("span", "server-chip", initials(server.name)));
    row.appendChild(el("span", null, server.name));
    row.onclick = () => selectServer(server.id);
    list.appendChild(row);
  }
  if (!servers.length) list.appendChild(el("div", "dm-preview", "No servers in common right now"));
  $("dm-profile-block").onclick = () => openBlock(userId, name);
}

// Stable per-user banner color so each profile looks distinct.
function bannerColor(userId) {
  let hash = 0;
  for (const ch of String(userId)) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  const hue = Math.abs(hash) % 360;
  return `linear-gradient(135deg, hsl(${hue} 42% 40%), hsl(${(hue + 40) % 360} 45% 26%))`;
}

// A role colour as a soft gradient, so a profile banner never reads as a flat block.
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

// The start-of-history block at the top of every DM.
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
    edited: false,
    attachments: [],
    pending: Boolean(m.pending),
  };
}

// Toast for a DM sent from outside the conversation. A pending DM is kept
// on this device and goes out once the other person is reachable.
function dmSentToast(sent, name, done) {
  toast(sent && sent.pending ? `Will send to ${name} when they come online` : done);
}

async function messageUser(userId, name) {
  if (state.user && userId === state.user.id) return;
  try {
    const thread = await invoke("open_dm", { userId });
    state.currentDm = { user: thread.user, name: thread.name };
    await enterDmMode();
  } catch (e) {
    toast(String(e));
  }
}

async function sendDm() {
  const input = $("message-input");
  const content = input.value.trim();
  if (!content || !state.currentDm) return;
  input.value = "";
  try {
    await invoke("send_dm", { userId: state.currentDm.user, content: encodeCustomEmojis(content) });
    await selectDm(state.currentDm.user, state.currentDm.name);
  } catch (e) {
    input.value = content;
    toast(String(e));
  }
}

async function selectChannel(channel) {
  state.currentChannel = channel;
  renderDmRequestBar(null);
  syncRulesGate();
  syncAwaitedBar();
  if (channel) clearChannelUnread(channel.id);
  renderChannels();
  $("channel-name").textContent = channel ? channel.name : "no channel";
  $("channel-topic").textContent = (channel && channel.topic) || "";
  $("message-input").placeholder = channel ? `Message #${channel.name}` : "Select a channel";
  if (!channel) return renderMessages([]);
  const messages = await invoke("get_messages", {
    serverId: state.currentServer.id,
    channelId: channel.id,
  });
  renderMessages(messages);
}

async function sendMessage() {
  if (state.view === "dm") return sendDm();
  const input = $("message-input");
  const content = input.value.trim();
  if (!content || !state.currentServer || !state.currentChannel) return;
  // "/gif cats" opens the GIF picker instead of sending text.
  const gif = content.match(/^\/gif(?:\s+(.*))?$/i);
  if (gif) {
    input.value = "";
    openGifPicker(gif[1] || "");
    return;
  }
  input.value = "";
  const replyTo = state.replyingTo ? state.replyingTo.id : null;
  $("messages").dataset.view = "";
  cancelReply();
  try {
    await invoke("send_message", {
      serverId: state.currentServer.id,
      channelId: state.currentChannel.id,
      content: encodeCustomEmojis(content),
      replyTo,
    });
    pushOverlayChat(state.user.id, state.user.display_name || state.user.username, encodeCustomEmojis(content), false);
    await selectChannel(state.currentChannel);
  } catch (e) {
    input.value = content;
    toast(String(e));
  }
}

// ---------- GIF search ----------

const GIF_KEY = "commy.gif";
const GIF_HELP = {
  klipy: 'To get a key, sign in at <a data-href="https://partner.klipy.com/api-keys">partner.klipy.com</a>, open API Keys, choose Add Platform, then Create API Key and copy it. Test keys are rate limited; use Request Production in the key menu for more.',
  giphy: 'To get a key, sign in at <a data-href="https://developers.giphy.com/dashboard/">developers.giphy.com</a>, choose Create an App, pick API, and copy the key.',
};
const GIF_PROVIDERS = new Set(["klipy", "giphy"]);

// The account server's discovery document says whether it searches GIFs.
// Old servers do not say, so the button stays and the picker explains.
function serverHasGifs() {
  return account.server_gifs !== false;
}

function syncGifButton() {
  $("gif-btn").classList.toggle("hidden", !gifSettings().key && !serverHasGifs());
}

function gifSettings() {
  let saved = {};
  try { saved = JSON.parse(localStorage.getItem(GIF_KEY) || "{}"); } catch { /* defaults */ }
  // Tenor was shut down; anything unknown falls back to KLIPY.
  if (!GIF_PROVIDERS.has(saved.provider)) saved = { provider: "klipy", key: "" };
  return { provider: "klipy", key: "", ...saved };
}

// KLIPY nests renditions as file.<size>.<format>.url (sizes xs, sm, md, hd).
function klipyUrl(file, sizes) {
  for (const size of sizes) {
    const url = file?.[size]?.gif?.url;
    if (url) return url;
  }
  return null;
}

// Height over width of the first KLIPY rendition that reports its size.
function klipyRatio(file, sizes) {
  for (const size of sizes) {
    const r = file?.[size]?.gif;
    if (r?.width && r?.height) return Number(r.height) / Number(r.width);
  }
  return 0;
}

// Returns [{ preview, url, title }] from the configured provider. Without a
// key of their own, people search through their account server's key.
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
    // KLIPY puts the key in the path rather than the query string.
    const base = `https://api.klipy.com/api/v1/${encodeURIComponent(settings.key)}/gifs`;
    const common = "per_page=24&page=1&content_filter=medium&format_filter=gif";
    const url = q ? `${base}/search?q=${q}&${common}` : `${base}/trending?${common}`;
    const res = await fetch(url);
    const body = await res.json().catch(() => ({}));
    // KLIPY reports problems as {result: false, errors: {message: [...]}}.
    if (!res.ok || body.result === false) {
      const reason = body?.errors?.message?.[0];
      throw new Error(reason || `KLIPY answered ${res.status}`);
    }
    const items = Array.isArray(body?.data) ? body.data : body?.data?.data || [];
    return items.map(item => ({
      preview: klipyUrl(item.file, ["sm", "xs", "md"]),
      ratio: klipyRatio(item.file, ["sm", "xs", "md"]),
      // md is a good chat size; hd can be over 10 MB.
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

let gifSearchToken = 0;

async function renderGifResults(query) {
  const grid = $("gif-grid");
  const token = ++gifSearchToken;
  grid.replaceChildren(el("div", "gif-note", "Searching..."));
  const settings = gifSettings();
  $("gif-attrib").textContent = settings.key && settings.provider === "giphy" ? "Powered by GIPHY" : "Powered by KLIPY";
  try {
    const gifs = await searchGifs(query, settings);
    if (token !== gifSearchToken) return;
    grid.replaceChildren();
    // Masonry: two columns, each GIF at its own aspect ratio, placed in
    // whichever column is shorter so the columns stay even.
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
      btn.onclick = () => sendGif(gif.url);
      const target = heights[0] <= heights[1] ? 0 : 1;
      cols[target].appendChild(btn);
      heights[target] += ratio;
    }
    if (gifs.length) grid.append(...cols);
    if (!gifs.length) grid.appendChild(el("div", "gif-note", "No GIFs found."));
  } catch (e) {
    if (token !== gifSearchToken) return;
    const note = el("div", "gif-note");
    if (e.message === "no-key") {
      note.textContent = "Your account server does not offer GIF search. Add your own GIF API key to search. ";
      const link = el("button", "btn-link", "Open GIF settings");
      link.onclick = () => {
        closeGifPicker();
        openUserSettings("gifs");
      };
      note.appendChild(link);
    } else {
      note.textContent = settings.key
        ? `Could not load GIFs: ${e.message}. Check your API key in Settings, then GIFs.`
        : `Could not load GIFs: ${e.message}.`;
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

async function toggleVoice(channel) {
  if (!state.currentServer) return;
  const leaving = state.voiceChannelId === channel.id;
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
      : { id: state.currentServer.id, name: state.currentServer.name, channelName: channel.name };
    if (leaving) {
      state.sharing = false;
      state.shareSource = null;
    }
    toast(leaving ? "Disconnected from voice" : `Joined voice: ${channel.name}`);
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

// ---------- Share picker ----------

const SHARE_QUALITY_KEY = "commy.shareQuality";
const SHARE_QUALITY_HINTS = {
  smooth: "Best for games and video. Lower resolution, more frames.",
  balanced: "A good default for most screens.",
  sharp: "Best for code and documents. Crisp text, fewer frames.",
};

const sharePicker = { sources: [], kind: "monitor", picked: null, thumbs: new Map(), token: 0 };

function shareQuality() {
  try {
    const saved = localStorage.getItem(SHARE_QUALITY_KEY);
    if (saved in SHARE_QUALITY_HINTS) return saved;
  } catch { /* storage unavailable */ }
  return "balanced";
}

function setShareQuality(quality) {
  try { localStorage.setItem(SHARE_QUALITY_KEY, quality); } catch { /* storage unavailable */ }
  for (const btn of $("share-quality").querySelectorAll("button")) {
    btn.classList.toggle("active", btn.dataset.q === quality);
  }
  $("share-quality-hint").textContent = SHARE_QUALITY_HINTS[quality];
}

// Pick what to stream; also used to switch source mid-stream.
async function openSharePicker() {
  if (!state.voiceServer) return;
  const token = ++sharePicker.token;
  sharePicker.sources = [];
  sharePicker.picked = null;
  sharePicker.thumbs.clear();
  sharePicker.kind = "monitor";
  setShareQuality(shareQuality());
  $("share-go").textContent = state.sharing ? "Switch Stream" : "Go Live";
  const list = $("share-source-list");
  list.innerHTML = "";
  list.appendChild(el("div", "share-loading", "Looking for screens and windows..."));
  renderShareTabs();
  showModal("modal-share");
  try {
    sharePicker.sources = await invoke("list_share_sources");
    if (token !== sharePicker.token) return;
    // A game you are playing goes first and is preselected.
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

// Thumbnails load one at a time so the picker opens instantly.
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
    } catch { /* window closed or protected; keep the icon */ }
  }
}

function goLive() {
  const source = sharePicker.picked;
  if (!source || source.kind !== sharePicker.kind) return;
  startShareWith(source, shareQuality());
}

async function startShareWith(source, quality) {
  hideModals();
  sharePicker.token++;
  try {
    // Switching source restarts the stream.
    if (state.sharing) await invoke("stop_screen_share", { serverId: state.voiceServer.id });
    await invoke("start_screen_share", {
      serverId: state.voiceServer.id,
      sourceKind: source.kind,
      sourceId: source.id,
      quality,
    });
    state.sharing = true;
    state.shareSource = source.name;
    state.shareQuality = quality;
    toast(`You are live: ${source.name}`);
    renderShareButton();
    startOwnPreview();
  } catch (e) {
    toast(String(e));
  }
}

// ---------- Own stream preview ----------

let ownPreviewTimer = null;
let ownFpsSample = null;

function startOwnPreview() {
  if (ownPreviewTimer) return;
  ownFpsSample = null;
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
      if (own.jpeg) {
        const src = `data:image/jpeg;base64,${own.jpeg}`;
        $("vp-preview-img").src = src;
        $("vp-preview-empty").classList.add("hidden");
        if (state.stageMode === "self") showStageFrame(src, own.width, own.height);
      }
      $("vp-viewers").textContent = own.viewers === 1 ? "1 watching" : `${own.viewers} watching`;
      $("vp-viewers").classList.toggle("hidden", !own.viewers);
      if (state.stageMode === "self") {
        $("stage-viewers").textContent = own.viewers === 1 ? "1 watching" : `${own.viewers} watching`;
        $("stage-viewers").classList.toggle("hidden", !own.viewers);
      }
      // Live encode rate from the frame counter between ticks.
      const now = performance.now();
      if (ownFpsSample && own.frames >= ownFpsSample.frames) {
        const fps = Math.round((own.frames - ownFpsSample.frames) / ((now - ownFpsSample.at) / 1000));
        $("vp-live-source").textContent = [state.shareSource ? `Streaming ${state.shareSource}` : "", `${fps} fps`].filter(Boolean).join(" - ");
      }
      ownFpsSample = { frames: own.frames, at: now };
    } catch { /* stream stopped between ticks */ }
  };
  tick();
  ownPreviewTimer = setInterval(tick, 1000);
}

// ---------- Stage (stream viewer) ----------

const stageStats = { times: [], width: 0, height: 0 };

// A stream whose frames stop (sharer left, crashed or lost network) ends
// for the viewer after a few seconds instead of freezing on the last frame.
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

// Shares a pasted or dropped File object into the current channel.
async function uploadFileObject(file) {
  if (state.view !== "server" || !state.currentServer || !state.currentChannel) {
    toast("Open a server channel to share files");
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
    } catch (e) { /* already gone */ }
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

// ---------- Event wiring ----------

function wireEvents() {
  $("signin-go").onclick = beginSignIn;
  $("signin-reopen").onclick = () => {
    if (signInUrl) invoke("open_url", { url: signInUrl }).catch(e => toast(String(e)));
  };
  $("signin-cancel").onclick = async () => {
    await invoke("account_cancel_sign_in");
    showSignIn();
  };
  $("signin-server-select").addEventListener("change", async e => {
    const value = e.target.value;
    if (value === "custom") {
      $("signin-server-edit").classList.remove("hidden");
      $("signin-server-input").value = "";
      $("signin-server-input").focus();
      return;
    }
    try {
      account = await invoke("set_account_server", { server: value === "default" ? null : value });
      showSignIn();
    } catch (err) {
      showSignInError(String(err));
    }
  });
  $("signin-server-input").addEventListener("keydown", e => {
    if (e.key === "Enter") $("signin-server-save").click();
  });
  $("signin-server-save").onclick = async () => {
    try {
      account = await invoke("set_account_server", { server: $("signin-server-input").value || null });
      showSignIn();
    } catch (e) {
      showSignInError(String(e));
    }
  };
  $("acct-sign-out").onclick = async () => {
    if (!confirm("Sign out of your Commy account on this device?")) return;
    try {
      account = await invoke("account_sign_out");
    } catch (e) {
      toast(String(e));
    }
  };
  $("username-save").onclick = async () => {
    const username = $("username-input").value.trim();
    if (!username) return;
    try {
      state.user = await invoke("init_user", { username });
      hideModals();
      renderUserPanel();
      await refreshServers();
      if (state.servers.length) await selectServer(state.servers[0].id);
      else await enterDmMode();
    } catch (e) {
      toast(String(e));
    }
  };
  $("username-input").addEventListener("keydown", e => {
    if (e.key === "Enter") $("username-save").click();
  });

  $("add-server-btn").onclick = () => showModal("modal-add-server");

  $("create-server-save").onclick = async () => {
    const name = $("create-name-input").value.trim();
    if (!name) return;
    try {
      const server = await invoke("create_server", { name });
      hideModals();
      $("create-name-input").value = "";
      await refreshServers();
      await selectServer(server.id);
    } catch (e) {
      toast(String(e));
    }
  };

  $("join-server-save").onclick = async () => {
    const ticket = $("join-ticket-input").value.trim();
    if (!ticket) return;
    if (looksLikeInviteLink(ticket)) {
      $("join-ticket-input").value = "";
      return openInviteLink(ticket);
    }
    try {
      const summary = await invoke("join_server", { ticket });
      hideModals();
      $("join-ticket-input").value = "";
      rolePromptPending.add(summary.id);
      await refreshServers();
      await selectServer(summary.id);
      toast("Joined server, syncing from peers");
    } catch (e) {
      toast(String(e));
    }
  };

  $("invite-btn").onclick = async () => {
    if (!state.currentServer) return;
    try {
      const invite = await invoke("get_invite", { serverId: state.currentServer.id });
      $("invite-text").value = invite;
      showModal("modal-invite");
    } catch (e) {
      toast(String(e));
    }
  };

  $("invite-copy").onclick = async () => {
    await navigator.clipboard.writeText($("invite-text").value);
    toast("Invite copied to clipboard");
  };

  $("settings-btn").onclick = () => openUserSettings("voice");
  $("settings-close").onclick = closeSettingsLayers;
  $("server-settings-close").onclick = closeSettingsLayers;
  for (const layer of document.querySelectorAll(".settings-layer")) {
    for (const btn of layer.querySelectorAll(".settings-tab[data-tab]")) {
      btn.onclick = () => selectSettingsTab(layer.id, btn.dataset.tab);
    }
  }

  // Appearance
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

  // Emoji picker
  $("emoji-btn").onclick = () => {
    const picker = $("emoji-picker");
    const opening = picker.classList.contains("hidden");
    picker.classList.toggle("hidden");
    if (opening) {
      renderEmojiGrid("");
      $("emoji-search").value = "";
      $("emoji-search").focus();
    }
  };
  $("emoji-search").addEventListener("input", e => renderEmojiGrid(e.target.value));
  document.addEventListener("mousedown", e => {
    const picker = $("emoji-picker");
    if (!picker.classList.contains("hidden") &&
        !picker.contains(e.target) && e.target !== $("emoji-btn") && !$("emoji-btn").contains(e.target)) {
      picker.classList.add("hidden");
    }
  });

  // Server settings
  $("server-settings-btn").onclick = () => openServerSettings();
  $("emoji-upload").onclick = uploadEmoji;
  $("sound-upload").onclick = uploadSound;
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
  $("sound-open-toggle").addEventListener("change", saveExpressionSettings);

  // Soundboard
  $("vp-soundboard").replaceChildren(svgIcon("soundboard", 18));
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
  $("new-role-save").onclick = async () => {
    if (!state.currentServer) return;
    const name = $("new-role-input").value.trim();
    if (!name) return;
    const permissions = Array.from($("new-role-perms").querySelectorAll("input:checked"))
      .map(box => box.value);
    try {
      await invoke("create_role", {
        serverId: state.currentServer.id,
        name,
        color: $("new-role-color").value,
        permissions,
      });
      $("new-role-input").value = "";
      for (const box of $("new-role-perms").querySelectorAll("input:checked")) box.checked = false;
      await refreshCurrentServer();
      openServerSettings();
    } catch (e) {
      toast(String(e));
    }
  };

  $("leave-server-btn").onclick = () => state.currentServer && openLeaveServer(state.currentServer.id);

  // Account
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
        customStatus: $("account-status").value || null,
        presence: $("account-presence").value,
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

  $("attach-btn").onclick = async () => {
    if (!state.currentServer || !state.currentChannel) return;
    try {
      const sent = await invoke("attach_file", {
        serverId: state.currentServer.id,
        channelId: state.currentChannel.id,
      });
      if (sent) await selectChannel(state.currentChannel);
    } catch (e) {
      toast(String(e));
    }
  };
  $("share-btn").onclick = toggleShare;
  $("video-close").onclick = closeStageButton;
  setIcon($("video-close"), "close", 20);
  setIcon($("stage-focus"), "expand", 20);
  setIcon($("stage-fullscreen"), "fullscreen", 20);
  $("stage-focus").onclick = toggleStageFocus;
  $("stage-fullscreen").onclick = toggleStageFullscreen;
  $("stage-view").ondblclick = toggleStageFullscreen;
  $("vp-preview").onclick = openOwnPreview;

  // Share picker
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

  // Profile popover
  setIcon($("pp-avatar-edit"), "pencil", 16);
  setIcon($("pp-clear"), "close", 14);
  $("pp-avatar-edit").onclick = e => {
    e.stopPropagation();
    openAvatarMenu(e.currentTarget);
  };
  $("pp-emoji").onclick = e => {
    e.stopPropagation();
    openEmojiPop(e.currentTarget, emoji => saveOwnProfile({ statusEmoji: emoji }));
  };
  $("pp-status").addEventListener("keydown", e => {
    if (e.key === "Enter") e.target.blur();
    if (e.key === "Escape") {
      e.target.value = state.user.custom_status || "";
      e.target.blur();
    }
  });
  $("pp-status").addEventListener("blur", e => {
    const value = e.target.value.trim();
    if (value !== (state.user.custom_status || "")) saveOwnProfile({ customStatus: value || null });
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

  // Profile card composer
  $("mc-emoji").replaceChildren(svgIcon("smile", 20));
  wireMentions($("mc-input"));
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
  $("account-about").addEventListener("change", async e => {
    try {
      state.user = await invoke("set_about", { about: e.target.value });
      toast("About Me saved");
    } catch (err) {
      toast(String(err));
    }
  });

  // Safety
  $("overlay-enabled").addEventListener("change", e => saveOverlaySettings({ enabled: e.target.checked }));
  $("overlay-corner").addEventListener("change", e => saveOverlaySettings({ corner: e.target.value }));
  $("overlay-opacity").addEventListener("input", e => {
    $("overlay-opacity-label").textContent = `${e.target.value}%`;
  });
  $("overlay-opacity").addEventListener("change", e => saveOverlaySettings({ opacity: Number(e.target.value) / 100 }));
  $("overlay-chat").addEventListener("change", e => saveOverlaySettings({ show_chat: e.target.checked }));
  wireHotkeyInput($("overlay-toggle-key"), "toggle_hotkey");
  wireHotkeyInput($("overlay-chat-key"), "chat_hotkey");
  $("dm-policy").addEventListener("change", saveDmSettings);
  $("spam-filter-toggle").addEventListener("change", saveDmSettings);
  $("block-cancel").onclick = hideModals;
  $("block-go").onclick = async () => {
    hideModals();
    if (blockTarget) await setBlocked(blockTarget.userId, blockTarget.name, true);
  };
  $("report-cancel").onclick = hideModals;
  $("report-go").onclick = submitReport;
  $("mm-message").onclick = () => {
    closeFloating();
    if (menuMember) messageUser(menuMember.id, menuMember.name);
  };
  $("mm-block").onclick = () => {
    closeFloating();
    if (!menuMember) return;
    if (isBlocked(menuMember.id)) setBlocked(menuMember.id, menuMember.name, false);
    else openBlock(menuMember.id, menuMember.name);
  };
  $("mm-report").onclick = () => {
    closeFloating();
    if (menuMember && state.currentServer) {
      openReport({ serverId: state.currentServer.id, userId: menuMember.id, name: menuMember.name });
    }
  };
  $("dm-request-accept").onclick = () => answerDmRequest("accept");
  $("dm-request-decline").onclick = () => answerDmRequest("decline");
  $("dm-request-block").onclick = () => answerDmRequest("block");
  $("pp-edit-profile").onclick = () => {
    closeFloating();
    openUserSettings("account");
  };

  // Avatar menu
  $("account-avatar-btn").onclick = e => {
    e.stopPropagation();
    openAvatarMenu(e.currentTarget);
  };
  $("avatar-upload").onclick = async () => {
    $("avatar-menu").classList.add("hidden");
    try {
      const src = await chooseImage("Choose your avatar", $("avatar-file"));
      if (src) await applyAvatar("image", await imageFileToAvatar(src));
    } catch (err) {
      toast(String(err));
    }
  };
  $("avatar-emoji").onclick = e => {
    e.stopPropagation();
    $("avatar-menu").classList.add("hidden");
    openEmojiPop(avatarMenuAnchor || e.currentTarget, emoji => applyAvatar("emoji", emoji));
  };
  $("avatar-remove").onclick = () => {
    $("avatar-menu").classList.add("hidden");
    applyAvatar("none");
  };
  $("emoji-pop-search").addEventListener("input", e => renderEmojiPop(e.target.value));
  // Clicks outside close floating menus; clicks inside keep them open.
  document.addEventListener("mousedown", e => {
    const open = ["emoji-pop", "avatar-menu", "profile-pop", "server-menu", "folder-menu", "member-menu", "pp-durations", "member-card", "invite-menu", "audio-pop", "conn-pop"].map($).filter(n => !n.classList.contains("hidden"));
    if (!open.length) return;
    if (open.some(n => n.contains(e.target)) || e.target.closest("#user-chip, .caret-btn, .vp-status")) return;
    // The emoji picker and mention list opened from the card keep it open.
    if (e.target.closest("#emoji-pop, #mention-pop, #gif-picker")) return;
    closeFloating();
  });
  document.addEventListener("keydown", e => {
    if (e.key === "Escape") closeFloating();
  });

  // Replies and threads
  setIcon($("reply-bar-cancel"), "close", 16);
  $("reply-bar-cancel").onclick = cancelReply;
  $("awaited-icon").replaceChildren(svgIcon("clock", 16));
  setIcon($("thread-close"), "close", 20);
  $("thread-head-icon").appendChild(svgIcon("thread", 20));
  $("thread-close").onclick = closeThread;
  $("thread-input").addEventListener("keydown", e => {
    if (e.key === "Enter") sendThreadReply();
    if (e.key === "Escape") closeThread();
  });
  $("message-input").addEventListener("keydown", e => {
    if (e.key === "Escape" && state.replyingTo) cancelReply();
  });

  // GIFs
  syncGifButton();
  $("gif-btn").onclick = () => ($("gif-picker").classList.contains("hidden") ? openGifPicker("") : closeGifPicker());
  $("gif-search").addEventListener("input", e => {
    clearTimeout(gifSearchTimer);
    gifSearchTimer = setTimeout(() => renderGifResults(e.target.value), 300);
  });
  $("gif-search").addEventListener("keydown", e => {
    if (e.key === "Escape") closeGifPicker();
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
    try { localStorage.setItem(GIF_KEY, JSON.stringify(settings)); } catch { /* session only */ }
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

  // Server and folder menus
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
  $("sm-read").onclick = () => {
    if (menuServer) markServerRead(menuServer.id);
    closeFloating();
  };
  $("sm-invite").onclick = async () => {
    closeFloating();
    if (!menuServer) return;
    if (!state.currentServer || state.currentServer.id !== menuServer.id) await selectServer(menuServer.id);
    $("invite-btn").click();
  };
  const fromMenu = action => async () => {
    closeFloating();
    if (!menuServer) return;
    if (!state.currentServer || state.currentServer.id !== menuServer.id) await selectServer(menuServer.id);
    action();
  };
  $("sm-events").onclick = fromMenu(openEvents);
  $("sm-rules").onclick = fromMenu(openRules);
  $("sm-roles").onclick = fromMenu(openChooseRoles);
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
  $("igdb-remove").onclick = () => saveIgdb(true);
  for (const link of document.querySelectorAll(".integration-steps a[data-href]")) {
    link.href = "#";
    link.onclick = e => {
      e.preventDefault();
      invoke("open_url", { url: link.dataset.href }).catch(err => toast(String(err)));
    };
  }
  $("rules-input").addEventListener("input", syncRulesPreview);
  $("rules-save").onclick = () => saveRules(false);
  $("rules-remove").onclick = () => saveRules(true);
  $("rules-close").onclick = hideModals;
  $("rules-accept").onclick = acceptRules;
  $("rules-gate-open").onclick = () => openServerWelcome("rules");
  $("sw-later").onclick = hideModals;
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
  $("sm-settings").onclick = async () => {
    closeFloating();
    if (!menuServer) return;
    if (!state.currentServer || state.currentServer.id !== menuServer.id) await selectServer(menuServer.id);
    openServerSettings();
  };
  $("sm-ungroup").onclick = () => {
    closeFloating();
    if (!menuServer) return;
    const layout = removeFromLayout(loadRailLayout(), menuServer.id);
    layout.push({ type: "server", id: menuServer.id });
    saveRailLayout(normalizeRailLayout(layout, state.servers.map(s => s.id)));
    renderServerRail();
  };
  $("sm-leave").onclick = () => {
    closeFloating();
    if (menuServer) openLeaveServer(menuServer.id);
  };
  $("leave-server-go").onclick = confirmLeaveServer;
  $("leave-server-cancel").onclick = hideModals;
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

  // Server images
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

  // Notifications
  $("notify-level").addEventListener("change", e => {
    const prefs = notifyPrefs();
    prefs.level = e.target.value;
    saveNotifyPrefs(prefs);
  });
  $("close-to-tray-toggle").addEventListener("change", async e => {
    try {
      state.voice = await invoke("set_close_to_tray", { enabled: e.target.checked });
    } catch (err) {
      toast(String(err));
    }
  });
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
  // Coming back to the window clears what you are looking at.
  window.addEventListener("focus", () => {
    if (state.view === "server" && state.currentChannel) clearChannelUnread(state.currentChannel.id);
    if (state.view === "dm" && state.currentDm) clearDmUnread(state.currentDm.user);
  });

  $("vp-camera").onclick = () => (state.cameraOn ? stopCamera() : startCamera());
  $("cam-preview-btn").onclick = toggleCameraPreview;
  $("camera-device").addEventListener("change", async e => {
    try { localStorage.setItem(CAMERA_KEY, e.target.value); } catch { /* per session only */ }
    if (previewStream) {
      stopCameraPreview();
      toggleCameraPreview();
    }
    // Switch a live camera to the new device.
    if (state.cameraOn) {
      await stopCamera();
      await startCamera();
    }
  });
  $("input-device").addEventListener("change", saveAudioDevices);
  $("output-device").addEventListener("change", saveAudioDevices);

  // Mentions
  wireMentions($("message-input"));
  wireMentions($("thread-input"));

  // Message actions
  $("delete-message-go").onclick = () => pendingDelete && deleteMessage(pendingDelete);
  $("delete-message-cancel").onclick = hideModals;
  $("message-input").addEventListener("keydown", e => {
    // Up arrow in an empty composer edits your last message.
    if (e.key !== "ArrowUp" || e.target.value || state.view !== "server") return;
    const mine = (state.renderedMessages || []).filter(m => isOwnMessage(m) && m.content && !isHidden(m.id));
    if (!mine.length) return;
    e.preventDefault();
    startEditMessage(mine[mine.length - 1].id);
  });

  // Channel settings
  $("channel-edit-name").addEventListener("input", syncChannelDirty);
  $("channel-edit-name").addEventListener("keydown", e => {
    if (e.key === "Enter") saveChannelEdits();
  });
  $("channel-edit-retention").addEventListener("change", syncChannelDirty);
  $("channel-limit").addEventListener("input", () => {
    syncLimitLabel();
    syncChannelDirty();
  });
  $("channel-edit-save").onclick = saveChannelEdits;
  $("channel-edit-reset").onclick = () => openChannelSettings(editingChannelId);
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

  // Create channel dialog
  for (const opt of $("create-channel-kinds").querySelectorAll(".kind-option")) {
    opt.onclick = () => setCreateChannelKind(opt.dataset.kind);
  }
  $("create-voice-icon").appendChild(svgIcon("speaker", 22));
  $("create-channel-name").addEventListener("input", syncCreateChannelName);
  $("create-channel-name").addEventListener("keydown", e => {
    if (e.key === "Enter" && !$("create-channel-save").disabled) saveCreateChannel();
  });
  $("create-channel-save").onclick = saveCreateChannel;
  $("create-channel-cancel").onclick = hideModals;

  // Voice panel
  $("vp-share").onclick = toggleShare;
  $("vp-stop-share").onclick = toggleShare;
  $("vp-change-source").onclick = openSharePicker;
  $("vp-stop-watch").onclick = stopWatching;
  $("vp-disconnect").onclick = disconnectVoice;
  // "Voice Connected" opens connection details; the channel name below it navigates.
  document.querySelector(".vp-status").title = "Connection details";
  document.querySelector(".vp-status").addEventListener("click", e => {
    e.stopPropagation();
    if ($("conn-pop").classList.contains("hidden")) openConnPop();
    else closeConnPop();
  });
  $("vp-conn-meta").onclick = () => {
    if (state.voiceServer && (!state.currentServer || state.currentServer.id !== state.voiceServer.id)) {
      selectServer(state.voiceServer.id);
    }
  };
  $("vp-nc").onclick = async () => {
    try {
      state.voice = await invoke("set_noise_cancellation", { enabled: !state.voice.noise_cancellation });
      $("nc-toggle").checked = state.voice.noise_cancellation;
      toast(`Noise cancellation ${state.voice.noise_cancellation ? "on" : "off"}`);
      renderVoicePanel();
    } catch (e) {
      toast(String(e));
    }
  };

  // Direct messages
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
  $("dm-profile-btn").onclick = () => {
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
    if (!$("lightbox").classList.contains("hidden")) closeLightbox();
    else if (!$("modal-backdrop").classList.contains("hidden") && state.user) hideModals();
    else if (openSettingsLayerId()) closeSettingsLayers();
  });

  $("message-input").addEventListener("keydown", e => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  });

  // Paste an image or file into the composer to share it. An image
  // copied from a web page carries its source URL in the html
  // clipboard flavor: paste that URL so the message keeps its
  // provenance, instead of uploading an anonymous bitmap. Images
  // with no web source (screenshots, local files) upload as before.
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

  // Drag a file anywhere onto the chat to share it.
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

  // Volume sliders apply live and persist on release.
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
    // Keep the username modal mandatory; others close on backdrop click.
    if (e.target === e.currentTarget && state.user && account.signed_in) hideModals();
  });

  // Live updates pushed from the Rust side.
  if (IS_TAURI) {
    window.__TAURI__.event.listen("server-event", async event => {
      const serverId = event.payload;
      loadShareableEmojis();
      if (state.currentServer && serverId === state.currentServer.id) {
        await refreshCurrentServer();
      }
    });
    window.__TAURI__.event.listen("servers-changed", refreshServers);
    window.__TAURI__.event.listen("soundboard-play", event => onSoundboardPlay(event.payload));
    window.__TAURI__.event.listen("report-received", event => onReportReceived(event.payload));
    window.__TAURI__.event.listen("account-changed", onAccountChanged);
    window.__TAURI__.event.listen("sign-in-failed", event => showSignInError(String(event.payload)));
    window.__TAURI__.event.listen("friends-changed", async event => {
      await loadFriends();
      const p = event.payload || {};
      if (p.signal === "request" && friendState(p.user) === "incoming") {
        toast(`${p.name} sent you a friend request`);
        invoke("notify", { title: "Friend request", body: `${p.name} wants to be friends`, target: `dm:${p.user}` }).catch(() => {});
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
    window.__TAURI__.event.listen("banned", async event => {
      toast(`You were banned from ${event.payload.name || "a server"}`);
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
    });
    window.__TAURI__.event.listen("overlay-hello", () => {
      lastOverlayRoster = "";
      pushOverlayRoster(true);
    });
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
    // One of our waiting DMs went out: redraw, nothing new arrived.
    window.__TAURI__.event.listen("dm-sent", async event => {
      if (state.view !== "dm") return;
      if (state.currentDm && event.payload === state.currentDm.user) {
        await selectDm(state.currentDm.user, state.currentDm.name);
      } else {
        await renderDmThreads();
      }
    });
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
    window.__TAURI__.event.listen("afk-moved", async () => {
      toast("You were moved to AFK for inactivity");
      if (state.currentServer) {
        const afk = state.currentServer.afk_channel;
        state.voiceChannelId = afk || state.voiceChannelId;
        const afkChannel = state.currentServer.channels.find(c => c.id === afk);
        if (afkChannel && state.voiceServer) state.voiceServer.channelName = afkChannel.name;
        await refreshCurrentServer();
        renderVoicePanel();
      }
    });
    // Our presence changed outside the account page (automatic AFK).
    window.__TAURI__.event.listen("profile-changed", event => {
      state.user = event.payload;
      renderUserPanel();
      renderVoicePanel();
      // Expiry may have changed the presence or cleared the status.
      invoke("get_voice_settings").then(v => {
        state.voice = v;
        if (!$("profile-pop").classList.contains("hidden")) renderProfilePopover();
      }).catch(() => {});
    });
    window.__TAURI__.event.listen("camera-frame", event => showCameraFrame(event.payload));
    window.__TAURI__.event.listen("message-created", event => onMessageCreated(event.payload));
    window.__TAURI__.event.listen("messages-caught-up", event => onMessagesCaughtUp(event.payload));
    window.__TAURI__.event.listen("dm-received", event => onDmReceived(event.payload));
    window.__TAURI__.event.listen("notification-clicked", event => openNotificationTarget(event.payload));
    window.__TAURI__.event.listen("tray-toggle-mute", () => $("mute-btn").click());
    window.__TAURI__.event.listen("voice-speaking", event => {
      setSpeaking(event.payload.user, Boolean(event.payload.speaking));
    });
    window.__TAURI__.event.listen("video-frame", event => {
      const frame = event.payload;
      if (state.watchingUser && frame.user === state.watchingUser) {
        showStageFrame(`data:image/jpeg;base64,${frame.jpeg}`, frame.width, frame.height);
      }
    });
  }
}

async function stopMicTest() {
  try { await invoke("stop_mic_test"); } catch (e) { /* already stopped */ }
  state.micTesting = false;
  $("mic-test-btn").textContent = "Start Test";
}

function renderEmojiGrid(query) {
  const grid = $("emoji-grid");
  grid.innerHTML = "";
  const q = query.trim().toLowerCase();
  appendCustomEmojiSections(grid, q, e => insertIntoComposer(`:${e.name}: `));
  const matches = EMOJI_DATA.filter(([, name]) => !q || name.includes(q)).slice(0, 160);
  for (const [emoji, name] of matches) {
    const btn = el("button", null, emoji);
    btn.title = name;
    btn.onclick = () => insertIntoComposer(emoji);
    grid.appendChild(btn);
  }
  if (!grid.childElementCount) grid.appendChild(el("div", "dm-preview", "No matches"));
}

function insertIntoComposer(text) {
  const input = $("message-input");
  const pos = input.selectionStart ?? input.value.length;
  input.value = input.value.slice(0, pos) + text + input.value.slice(pos);
  input.focus();
  const next = pos + text.length;
  input.setSelectionRange(next, next);
}

const PERMISSION_LABELS = [
  ["manage_server", "Manage Server"],
  ["manage_channels", "Manage Channels"],
  ["manage_messages", "Manage Messages"],
  ["manage_expressions", "Manage Expressions"],
  ["manage_events", "Manage Events"],
  ["manage_roles", "Manage Roles"],
  ["kick_members", "Kick Members"],
  ["ban_members", "Ban Members"],
  ["mute_members", "Mute Members"],
  ["administrator", "Administrator"],
];

function openUserSettings(tab) {
  if (!state.user) return;
  $("nc-toggle").checked = state.voice.noise_cancellation;
  $("afk-timeout").value = String(state.voice.afk_timeout_minutes ?? 10);
  $("share-activity-toggle").checked = state.voice.share_activity !== false;
  loadAudioDevices();
  loadCameraDevices();
  renderSoundSettings();
  loadSafety().then(renderSafetySettings);
  $("account-about").value = state.user.about || "";
  loadOverlaySettings().then(renderOverlaySettings);
  loadIntegrations().then(renderIntegrations);
  loadAccount().then(() => {
    renderAccountSettings();
    // The GIF help depends on what the account server offers
    $("gif-provider").dispatchEvent(new Event("change"));
  });
  $("notify-level").value = notifyPrefs().level;
  $("close-to-tray-toggle").checked = state.voice.close_to_tray !== false;
  $("output-vol").value = Math.round((state.voice.output_volume ?? 1) * 100);
  $("input-gain").value = Math.round((state.voice.input_gain ?? 1) * 100);
  $("output-vol-label").textContent = `${$("output-vol").value}%`;
  $("input-gain-label").textContent = `${$("input-gain").value}%`;
  $("account-display-name").value = state.user.display_name || "";
  $("account-status").value = state.user.custom_status || "";
  $("account-presence").value = state.user.presence || "online";
  $("account-avatar").replaceChildren();
  paintAvatar($("account-avatar"), state.user.id, state.user.display_name || state.user.username);
  $("account-preview-name").textContent = state.user.display_name || state.user.username;
  $("account-username").textContent = state.user.username;
  openSettingsLayer("user-settings", tab);
}

// Re-opening keeps the current tab so actions inside a tab do not jump away.
function openServerSettings(tab) {
  if (!state.currentServer) return;
  const server = state.currentServer;
  const layer = $("server-settings");
  const canMembers = server.can_kick || server.can_manage_roles;
  $("server-settings-title").textContent = server.name;

  $("rename-input").value = server.name;
  $("rename-input").disabled = !server.can_manage_server;
  $("rename-save").classList.toggle("hidden", !server.can_manage_server);
  $("overview-readonly").classList.toggle("hidden", server.can_manage_server);
  const afkSelect = $("afk-channel-select");
  afkSelect.innerHTML = "";
  const none = el("option", null, "No AFK channel");
  none.value = "";
  afkSelect.appendChild(none);
  for (const channel of server.channels.filter(c => c.kind === "voice")) {
    const opt = el("option", null, channel.name);
    opt.value = channel.id;
    afkSelect.appendChild(opt);
  }
  afkSelect.value = server.afk_channel || "";
  afkSelect.disabled = !server.can_manage_server;
  $("server-afk-timeout").value = String(server.afk_timeout_minutes || 0);
  $("server-afk-timeout").disabled = !server.can_manage_server;
  paintServerImages(server);
  $("tab-channels").classList.toggle("hidden", !server.can_manage_channels);
  $("tab-roles").classList.toggle("hidden", !server.can_manage_roles);
  $("tab-members").classList.toggle("hidden", !canMembers);
  const canEmoji = server.can_manage_expressions || server.can_manage_server;
  const canSounds = canEmoji || server.can_upload_sounds;
  $("tab-rules").classList.toggle("hidden", !server.can_manage_server);
  renderRulesSettings(server);
  renderGameTagEditor(server);
  const isOwner = Boolean(state.user && server.owner === state.user.id);
  $("tab-reports").classList.toggle("hidden", !isOwner);
  if (isOwner) renderReportSettings(server);
  $("tab-emoji").classList.toggle("hidden", !canEmoji);
  $("tab-soundboard").classList.toggle("hidden", !canSounds);
  const allowed = {
    overview: true,
    channels: server.can_manage_channels,
    roles: server.can_manage_roles,
    emoji: canEmoji,
    soundboard: canSounds,
    reports: isOwner,
    rules: server.can_manage_server,
    members: canMembers,
  };
  renderEmojiSettings(server);
  renderSoundboardSettings(server);
  let current = tab || (layer.classList.contains("hidden") ? "overview" : layer.dataset.tab);
  if (!allowed[current]) current = "overview";

  const list = $("manage-channel-list");
  list.innerHTML = "";
  for (const channel of server.channels) {
    const row = el("div", "manage-row");
    const label = el("span", "manage-row-label");
    const icon = el("span", "channel-icon");
    if (channel.kind === "voice") icon.appendChild(svgIcon("speaker", 18));
    else icon.textContent = "#";
    label.appendChild(icon);
    label.appendChild(el("span", null, channel.name));
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

function renderRoleSettings(server) {
  const list = $("manage-role-list");
  list.innerHTML = "";
  for (const role of server.roles) {
    const row = el("div", "manage-row");
    const label = el("span", "manage-row-label");
    const dot = el("span", "role-dot");
    if (role.color) dot.style.background = `#${role.color}`;
    label.appendChild(dot);
    label.appendChild(document.createTextNode(role.name));
    if (role.tag) label.appendChild(roleTagChip(role.tag, role.color));
    if (role.self_assignable) label.appendChild(el("span", "kind pickable", "pickable"));
    label.appendChild(el("span", "kind", role.permissions.join(", ") || "no permissions"));
    row.appendChild(label);
    const edit = el("button", "btn-secondary btn-small", "Edit");
    edit.onclick = () => {
      const open = row.nextElementSibling && row.nextElementSibling.classList.contains("role-editor");
      if (open) row.nextElementSibling.remove();
      else row.after(roleEditor(server, role));
    };
    row.appendChild(edit);
    const del = el("button", "btn-danger btn-small", "Delete");
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
    row.appendChild(del);
    list.appendChild(row);
  }
  if (!server.roles.length) list.appendChild(el("div", "manage-empty", "No roles yet"));

  const grid = $("new-role-perms");
  if (!grid.childElementCount) {
    for (const [value, label] of PERMISSION_LABELS) {
      const wrap = el("label");
      const box = el("input");
      box.type = "checkbox";
      box.value = value;
      wrap.appendChild(box);
      wrap.appendChild(document.createTextNode(label));
      grid.appendChild(wrap);
    }
  }
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
    if (member.color) name.style.color = `#${member.color}`;
    who.appendChild(name);
    if (member.id === server.owner) who.appendChild(el("span", "kind", "owner"));
    top.appendChild(who);
    const isSelf = state.user && member.id === state.user.id;
    const isOwner = member.id === server.owner;
    if (server.can_kick && !isSelf && !isOwner) {
      const kick = el("button", "btn-danger btn-small", "Kick");
      kick.onclick = async () => {
        if (!confirm(`Kick ${member.name} from the server?`)) return;
        try {
          await invoke("kick_member", { serverId: server.id, userId: member.id });
          await refreshCurrentServer();
          openServerSettings();
        } catch (e) {
          toast(String(e));
        }
      };
      top.appendChild(kick);
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

// Re-render server data without resetting the selected channel or
// losing composer focus.
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
  syncRulesGate();
  syncAwaitedBar();
  renderEvents();
  if (keepChannel) {
    const still = state.currentServer.channels.find(c => c.id === keepChannel.id);
    if (still) {
      state.currentChannel = still;
      const messages = await invoke("get_messages", {
        serverId: state.currentServer.id,
        channelId: still.id,
      });
      renderMessages(messages);
      return;
    }
  }
  await selectChannel(state.currentServer.channels.find(c => c.kind === "text") || null);
}

// ---------- Boot ----------

// Tell the backend the user is at the keyboard, at most every 30s,
// so the AFK watchdog only moves people who are actually idle.
function wireActivityPing() {
  let lastPing = 0;
  const ping = () => {
    const now = Date.now();
    if (now - lastPing < 30e3) return;
    lastPing = now;
    invoke("note_user_activity").catch(() => {});
  };
  for (const evt of ["pointerdown", "pointermove", "keydown", "wheel"]) {
    window.addEventListener(evt, ping, { passive: true });
  }
}

async function boot() {
  wireEvents();
  wireActivityPing();
  if (IS_TAURI) {
    await invoke("ui_ready"); // handshake logged by the Rust side
  } else {
    toast("Preview mode: running on mock data, not the real backend");
  }
  state.voice = await invoke("get_voice_settings");
  await loadSafety();
  await loadFriends();
  await loadOverlaySettings();
  await loadIntegrations();
  await loadAccount();
  if (!account.signed_in) {
    showSignIn();
    return;
  }
  await continueBoot();
}

// Runs once signed in: pick a username if needed, then load servers.
async function continueBoot() {
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
  if (state.servers.length) await selectServer(state.servers[0].id);
  else await enterDmMode();
}

// ---------- Commy account ----------

let account = { signed_in: false, server_host: "" };

async function loadAccount() {
  try {
    account = await invoke("account_status");
  } catch {
    /* keep the last state */
  }
  syncGifButton();
}

let signInUrl = null;

function showSignIn() {
  $("signin-start").classList.remove("hidden");
  $("signin-wait").classList.add("hidden");
  $("signin-error").classList.add("hidden");
  const select = $("signin-server-select");
  const custom = !account.is_default_server;
  const defaultHost = account.default_server_host || "commy.chat";
  select.options[0].text = defaultHost === "commy.chat" ? "commy.chat (free Commy service)" : `${defaultHost} (test server)`;
  if (custom && ![...select.options].some(o => o.value === account.server)) {
    select.insertBefore(new Option(account.server_host, account.server), select.options[1]);
  }
  select.value = custom ? account.server : "default";
  select.disabled = Boolean(account.server_from_env);
  select.title = account.server_from_env ? "Set by COMMY_AUTH_URL" : "";
  $("signin-server-edit").classList.add("hidden");
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

async function onAccountChanged() {
  const was = account.signed_in;
  await loadAccount();
  if (account.signed_in && !was) {
    toast(`Signed in as ${(account.user && account.user.name) || "you"}`);
    await continueBoot();
  } else if (!account.signed_in && was) {
    showSignIn();
  }
  if (!$("user-settings").classList.contains("hidden")) renderAccountSettings();
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
