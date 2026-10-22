const ANSI = /\x1b\[[0-9;]*m/g;
const EPOCH_LINE = /^\s*(\d+)\.(\d{3})\s+(\d+)\s+(\d+)\s+([VDIWEF])\s+(.*)$/;

export const PATTERNS = {
  crash: /FATAL EXCEPTION|UnsatisfiedLinkError|panicked at|Fatal signal|tombstone/,
  firebaseReady: /FirebaseInitProvider.*initialization successful/,
  webviewConnected: /webview connected over the tauri bridge/,
  nodeOnline: /iroh node online/,
  wakeRegistered: /push_wake: registered (\d+) wake-up tags/,
  wakeOff: /the sync service does not offer wake-ups/,
  wakeReceived: /CommyWake.*wake-up push received, foreground=(true|false), notifications allowed=(true|false)/,
  wakeRang: /push_wake: rang (\d+) wake-up tags/,
  startedForPush: /Start proc (\d+):([\w.]+)\/\S+ for broadcast \{[\w.]+\/com\.google\.firebase\.iid\.FirebaseInstanceIdReceiver\}/,
  linkFailed: /could not link this device to the account yet/,
};

export function stripAnsi(text) {
  return text.replace(ANSI, "");
}

export function parseEpochLog(text) {
  const entries = [];
  for (const raw of stripAnsi(text).split(/\r?\n/)) {
    const m = EPOCH_LINE.exec(raw);
    if (!m) continue;
    entries.push({ at: Number(m[1]) * 1000 + Number(m[2]), pid: Number(m[3]), level: m[5], text: m[6] });
  }
  return entries;
}

export function since(entries, at) {
  return entries.filter(e => e.at >= at);
}

export function find(entries, pattern) {
  return entries.find(e => pattern.test(e.text)) ?? null;
}

export function findAll(entries, pattern) {
  return entries.filter(e => pattern.test(e.text));
}

export function crashes(entries, pids) {
  const ours = new Set(pids);
  return entries.filter(e => PATTERNS.crash.test(e.text) && (ours.size === 0 || ours.has(e.pid) || /chat\.commy/.test(e.text)));
}

export function registeredTags(entries) {
  const hit = [...entries].reverse().find(e => PATTERNS.wakeRegistered.test(e.text));
  return hit ? Number(PATTERNS.wakeRegistered.exec(hit.text)[1]) : null;
}

export function wakes(entries) {
  return findAll(entries, PATTERNS.wakeReceived).map(e => {
    const [, foreground, allowed] = PATTERNS.wakeReceived.exec(e.text);
    return { at: e.at, pid: e.pid, foreground: foreground === "true", notificationsAllowed: allowed === "true" };
  });
}

export function countActiveNotifications(dumpsys, pkg) {
  const blocks = dumpsys.split(/\n(?=\s*NotificationRecord\()/);
  return blocks.filter(b => b.includes(`pkg=${pkg}`) && !/GROUP_SUMMARY/.test(b.split("\n")[0])).length;
}
