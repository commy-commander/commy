export function relayOrigins(relay) {
  const url = new URL(relay);
  const socket = url.protocol === "https:" ? "wss:" : "ws:";
  return [`${url.protocol}//${url.host}`, `${socket}//${url.host}`];
}

export function contentSecurityPolicy(relays) {
  return [
    "default-src 'self'",
    "script-src 'self' 'wasm-unsafe-eval'",
    `connect-src 'self' ${relays.join(" ")}`,
    "img-src 'self' data: blob:",
    "media-src 'self' data: blob:",
    "style-src 'self' 'unsafe-inline'",
    "font-src 'self'",
    "frame-src 'none'",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
  ].join("; ");
}

export function webRelayList(raw, fallback) {
  const relays = (raw || "")
    .split(",")
    .map(relay => relay.trim().replace(/\/+$/, ""))
    .filter(Boolean);
  return relays.length ? relays : [fallback];
}

export function injectWebShell(html, { relays, base }) {
  const origins = relays.flatMap(relayOrigins);
  const head = [
    `<meta http-equiv="Content-Security-Policy" content="${contentSecurityPolicy(origins)}" />`,
    `<meta name="commy-relays" content="${relays[0]}" />`,
  ].join("\n  ");
  const based = base ? html.replace("<head>", `<head>\n  <base href="${base}" />`) : html;
  const withHead = based.replace(
    '<link rel="stylesheet" href="styles/styles.css" />',
    `${head}\n  <link rel="stylesheet" href="styles/styles.css" />\n  <link rel="stylesheet" href="styles/web.css" />`,
  );
  const firstScript = withHead.indexOf('<script src="scripts/');
  if (firstScript < 0 || withHead === based || (base && based === html)) throw new Error("index.html no longer has the expected stylesheet and script tags");
  const bridge = '<script src="scripts/web-commands.js"></script>\n  <script src="scripts/web-voice.js"></script>\n  <script src="scripts/web-bridge.js"></script>\n  ';
  return withHead.slice(0, firstScript) + bridge + withHead.slice(firstScript);
}
