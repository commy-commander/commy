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

export function injectWebShell(html, relays) {
  const csp = `<meta http-equiv="Content-Security-Policy" content="${contentSecurityPolicy(relays)}" />`;
  const withHead = html.replace(
    '<link rel="stylesheet" href="styles/styles.css" />',
    `${csp}\n  <link rel="stylesheet" href="styles/styles.css" />\n  <link rel="stylesheet" href="styles/web.css" />`,
  );
  const firstScript = withHead.indexOf('<script src="scripts/');
  if (firstScript < 0 || withHead === html) throw new Error("index.html no longer has the expected stylesheet and script tags");
  const bridge = '<script src="scripts/web-commands.js"></script>\n  <script src="scripts/web-bridge.js"></script>\n  ';
  return withHead.slice(0, firstScript) + bridge + withHead.slice(firstScript);
}
