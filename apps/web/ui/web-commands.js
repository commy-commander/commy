"use strict";

const WEB_VOICE_SETTINGS = {
  noise_cancellation: true, muted: false, deafened: false, output_volume: 1, input_gain: 1, afk_timeout_minutes: 10,
};

const WEB_UPDATE_STATUS = {
  current: "web", can_install: false, reason: "web", phase: "idle", version: null, progress: null, checked_at: null, error: null, page: null,
};

const WEB_DEFAULTS = {
  ui_ready: null,
  get_voice_settings: WEB_VOICE_SETTINGS,
  get_safety: { blocked: [], dm_policy: "everyone", spam_filter: true },
  list_friends: { friends: [], incoming: [], outgoing: [] },
  get_overlay_settings: { enabled: false, corner: "top-left", opacity: 0.9, show_chat: false, show_everyone: false, toggle_hotkey: "", chat_hotkey: "" },
  overlay_hotkey_problems: [],
  voice_hotkey_problems: [],
  get_update_status: WEB_UPDATE_STATUS,
  check_for_updates: WEB_UPDATE_STATUS,
  get_integrations: { igdb_client_id: "", igdb_configured: false },
  get_storage_settings: { keep: "90", servers: {} },
  get_autostart: false,
  get_stream_deck: { enabled: false, status: "off", decks: [] },
  list_reports: [],
  list_dm_threads: [],
  get_dm_messages: [],
  get_awaited: 0,
  note_user_activity: null,
  set_window_title: null,
  get_dm_call: null,
  set_unread: null,
  get_custom_icon: null,
  take_pending_invite_link: null,
  list_shareable_emojis: [],
  list_shareable_sounds: [],
  list_shareable_expressions: {},
  list_server_templates: [],
  list_audio_devices: { inputs: [], outputs: [] },
  voice_stats: null,
  notify: null,
  account_cancel_sign_in: null,
  search_messages: { total: 0, hits: [] },
  entry_list_status: { state: "synced", servers: 0, has_key: false, detail: "This browser opens your servers through your other devices." },
};

const WEB_SERVER_WAIT_MS = 20000;

const WEB_UNLOCK_TEXT = {
  signed_out: "Sign in to open your servers.",
  no_list: "Open Commy on your computer and sign in there first. The web app opens the servers your computer has.",
  locked: "Approve this browser on one of your Commy devices, or enter your recovery code.",
  asking: "Open Commy on one of your other devices and choose Allow.",
  opening: "Opening your servers",
  unlocked: "",
};

function webDefault(cmd) {
  if (!Object.prototype.hasOwnProperty.call(WEB_DEFAULTS, cmd)) return { known: false };
  return { known: true, value: structuredClone(WEB_DEFAULTS[cmd]) };
}

function webSignInUrl(origin) {
  return `${origin}/account`;
}

function webUnlockText(view) {
  return (view && view.detail) || WEB_UNLOCK_TEXT[view && view.state] || WEB_UNLOCK_TEXT.locked;
}

function webRelays(doc) {
  const meta = doc.querySelector('meta[name="commy-relays"]');
  const relays = meta ? meta.getAttribute("content").trim() : "";
  return relays || undefined;
}

function webReplyLine(status, text) {
  return `${status}\n${text}`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { WEB_DEFAULTS, WEB_SERVER_WAIT_MS, WEB_UNLOCK_TEXT, webDefault, webSignInUrl, webUnlockText, webReplyLine, webRelays };
}
