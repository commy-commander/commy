import dns from "node:dns/promises";
import https from "node:https";
import tls from "node:tls";
import { DISCOVERY_PATH, isHomeOrigin, publicOnlyLookup, publishedKeyOf } from "./home-keys.js";

export const SYNC_ABOUT_PATH = "/sync/about";
export const CHECK_IDS = ["address", "tls", "discovery", "version", "contact", "legal", "adults"];
const FETCH_TIMEOUT_MS = 10000;
const MAX_BODY_BYTES = 256 * 1024;
const CERT_DAYS_LEFT = 14;
const DAY_MS = 24 * 60 * 60 * 1000;
const EMAIL = /^[^\s@<>"]{1,64}@((?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{0,61}[a-z0-9])$/i;
const VERSION = /^v?(\d+)\.(\d+)\.(\d+)/;

export function versionParts(text) {
  const match = VERSION.exec(String(text ?? "").trim());
  return match ? match.slice(1, 4).map(Number) : null;
}

export function atLeast(version, oldest) {
  const have = versionParts(version);
  const need = versionParts(oldest);
  if (!have) return false;
  if (!need) return true;
  for (let i = 0; i < 3; i++) if (have[i] !== need[i]) return have[i] > need[i];
  return true;
}

export function oldestSupported(latest) {
  const parts = versionParts(latest);
  if (!parts) return null;
  const [major, minor] = parts;
  return minor > 0 ? `${major}.${minor - 1}.0` : `${major}.0.0`;
}

export function contactKind(contact) {
  const text = String(contact ?? "").trim();
  if (EMAIL.test(text)) return { kind: "email", value: text, domain: EMAIL.exec(text)[1].toLowerCase() };
  if (/^https:\/\/[^\s"<>]+$/.test(text)) return { kind: "page", value: text };
  return null;
}

export function cleanOrigin(input) {
  const text = String(input ?? "").trim().toLowerCase().replace(/\/+$/, "");
  const withScheme = /^[a-z]+:\/\//.test(text) ? text : `https://${text}`;
  return isHomeOrigin(withScheme) ? withScheme : null;
}

function absoluteHttps(link, origin) {
  if (!link) return null;
  try {
    const url = new URL(String(link), origin);
    return url.protocol === "https:" ? url.toString() : null;
  } catch {
    return null;
  }
}

function get(url, { accept = "application/json", lookup = publicOnlyLookup } = {}) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { lookup, timeout: FETCH_TIMEOUT_MS, headers: { accept, "user-agent": "Commy listing check" } }, (res) => {
      let size = 0;
      const chunks = [];
      res.on("data", (chunk) => {
        size += chunk.length;
        if (size > MAX_BODY_BYTES) return req.destroy(new Error("too large"));
        chunks.push(chunk);
      });
      res.on("end", () => resolve({ status: res.statusCode, type: String(res.headers["content-type"] ?? ""), body: Buffer.concat(chunks).toString("utf8") }));
    });
    req.on("timeout", () => req.destroy(new Error("timed out")));
    req.on("error", reject);
  });
}

async function fetchJson(url) {
  const res = await get(url);
  if (res.status !== 200) throw new Error(`answered ${res.status}`);
  try {
    return JSON.parse(res.body);
  } catch {
    throw new Error("did not answer with JSON");
  }
}

async function fetchPage(url) {
  const res = await get(url, { accept: "text/html" });
  return { status: res.status, type: res.type };
}

function certificate(host, { lookup = publicOnlyLookup } = {}) {
  return new Promise((resolve, reject) => {
    const socket = tls.connect({ host, port: 443, servername: host, lookup, timeout: FETCH_TIMEOUT_MS, rejectUnauthorized: false }, () => {
      const cert = socket.getPeerCertificate();
      const result = { valid: socket.authorized, problem: socket.authorizationError ? String(socket.authorizationError) : null, validTo: cert?.valid_to ? Date.parse(cert.valid_to) : null };
      socket.end();
      resolve(result);
    });
    socket.on("timeout", () => socket.destroy(new Error("timed out")));
    socket.on("error", reject);
  });
}

async function acceptsMail(domain) {
  const mx = await dns.resolveMx(domain).catch(() => []);
  if (mx.some((record) => record.exchange && record.exchange !== ".")) return true;
  const a = await dns.resolve(domain).catch(() => []);
  return a.length > 0;
}

export const liveProbes = { fetchJson, fetchPage, certificate, acceptsMail };

function pass(id, detail) {
  return { id, ok: true, detail };
}

function fail(id, detail) {
  return { id, ok: false, detail };
}

async function reachHome(probes, origin) {
  const document = await probes.fetchJson(`${origin}${DISCOVERY_PATH}`).catch(() => null);
  if (!document) return null;
  const key = publishedKeyOf(document, origin);
  return { document, key };
}

async function reachSync(probes, origin) {
  const about = await probes.fetchJson(`${origin}${SYNC_ABOUT_PATH}`).catch(() => null);
  return about && Number.isInteger(about.protocol) ? about : null;
}

async function checkTls(probes, origin, now) {
  const host = new URL(origin).hostname;
  const cert = await probes.certificate(host).catch((err) => ({ valid: false, problem: err.message, validTo: null }));
  if (!cert.valid) return fail("tls", `The certificate is not accepted (${cert.problem ?? "unknown problem"}).`);
  const daysLeft = cert.validTo ? Math.floor((cert.validTo - now) / DAY_MS) : null;
  if (daysLeft !== null && daysLeft < CERT_DAYS_LEFT) return fail("tls", `The certificate runs out in ${daysLeft} days. Renew it first.`);
  return pass("tls", daysLeft === null ? "The certificate is accepted." : `The certificate is accepted and valid for ${daysLeft} more days.`);
}

function checkDiscovery(home, sync) {
  if (home && !home.key) return fail("discovery", "The account server's discovery document does not name this address or its server key.");
  if (!home && !sync) return fail("discovery", "Neither an account server nor a sync host answers at this address.");
  const kinds = [home && "accounts", sync && "communities"].filter(Boolean).join(" and ");
  return pass("discovery", `This server hosts ${kinds}.`);
}

function checkVersion(home, sync, oldest) {
  const reported = [home && { what: "The account server", version: home.document.release }, sync && { what: "The sync host", version: sync.version }].filter(Boolean);
  if (reported.length === 0) return fail("version", "There is no server to read a version from.");
  for (const { what, version } of reported) {
    if (!versionParts(version)) return fail("version", `${what} does not report its release. Update to the latest release.`);
    if (!atLeast(version, oldest)) return fail("version", `${what} runs ${version}; listed servers need ${oldest} or newer.`);
  }
  return pass("version", `Runs ${reported.map((r) => r.version).join(" and ")}.`);
}

async function checkContact(probes, home, sync) {
  const raw = home?.document.abuseContact ?? sync?.abuse_contact ?? null;
  const contact = contactKind(raw);
  if (!contact) return { result: fail("contact", "No abuse contact is set. Set COMMY_ABUSE_CONTACT or SYNC_ABUSE_CONTACT."), contact: null };
  if (contact.kind === "email") {
    const ok = await probes.acceptsMail(contact.domain).catch(() => false);
    return { result: ok ? pass("contact", `Abuse reports go to ${contact.value}.`) : fail("contact", `${contact.domain} does not accept email.`), contact };
  }
  const page = await probes.fetchPage(contact.value).catch(() => null);
  const ok = page?.status === 200;
  return { result: ok ? pass("contact", `Abuse reports go to ${contact.value}.`) : fail("contact", `The abuse contact page ${contact.value} does not load.`), contact };
}

async function checkLegal(probes, links) {
  const missing = ["terms", "privacy"].filter((name) => !links[name]);
  if (missing.length) return fail("legal", `Link your own ${missing.join(" and ")} page.`);
  for (const name of ["terms", "privacy"]) {
    const page = await probes.fetchPage(links[name]).catch(() => null);
    if (page?.status !== 200 || !page.type.includes("text/html")) return fail("legal", `The ${name} page ${links[name]} does not load.`);
  }
  return pass("legal", "The Terms and privacy pages load.");
}

function checkAdults(home) {
  if (!home) return pass("adults", "Communities only: accounts and the 18+ check stay with each person's account server.");
  return home.document.features?.adultsOnly === true ? pass("adults", "The 18+ check is on.") : fail("adults", "The 18+ check is off. Set ADULTS_ONLY=true.");
}

export async function checkServer({ origin: input, terms = null, privacy = null }, { probes = liveProbes, blocked = () => false, oldest = null, now = Date.now() } = {}) {
  const origin = cleanOrigin(input);
  if (!origin) return { origin: null, passed: false, checks: [fail("address", "Use an https address on a public domain, like https://chat.example.org.")] };
  const domain = new URL(origin).hostname;
  if (blocked(domain)) return { origin, passed: false, checks: [fail("address", `${domain} is on Commy's list of blocked servers.`)] };

  const tlsResult = await checkTls(probes, origin, now);
  const [home, sync] = await Promise.all([reachHome(probes, origin), reachSync(probes, origin)]);
  const links = home
    ? { terms: absoluteHttps(home.document.legal?.terms, origin), privacy: absoluteHttps(home.document.legal?.privacy, origin) }
    : { terms: absoluteHttps(terms, origin), privacy: absoluteHttps(privacy, origin) };
  const { result: contactResult, contact } = await checkContact(probes, home, sync);
  const checks = [
    pass("address", `${domain} is a public address.`),
    tlsResult,
    checkDiscovery(home, sync),
    checkVersion(home, sync, oldest),
    contactResult,
    await checkLegal(probes, links),
    checkAdults(home),
  ];
  const kinds = [home?.key && "home", sync && "sync"].filter(Boolean);
  return {
    origin,
    passed: checks.every((check) => check.ok) && kinds.length > 0,
    checks,
    kinds,
    name: typeof home?.document.name === "string" ? home.document.name : typeof sync?.operator === "string" ? sync.operator : null,
    key: home?.key ?? null,
    contact: contact?.value ?? null,
    contactKind: contact?.kind ?? null,
    terms: links.terms,
    privacy: links.privacy,
  };
}

export function oldestFor(config) {
  return config.listing.oldestVersion ?? oldestSupported(config.release);
}

export function createChecker({ oldest = null, blocked = () => false, probes = liveProbes, now = () => Date.now() } = {}) {
  return { check: (application) => checkServer(application, { probes, blocked, oldest, now: now() }) };
}
