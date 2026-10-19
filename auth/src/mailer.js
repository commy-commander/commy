import { readFileSync } from "node:fs";
import nodemailer from "nodemailer";

const ICON_CID = "icon@commy";
const ICON = readFileSync(new URL("../assets/email-icon.png", import.meta.url));

const COLOR = {
  ground: "#0b0d17",
  panel: "#151929",
  line: "#1f2438",
  heading: "#f2efe6",
  text: "#e2e0da",
  muted: "#a9aebf",
  faint: "#878da5",
  accent: "#ff4d4d",
  accentInk: "#ffffff",
};

const TEXT_FONT = "Figtree,'Segoe UI',-apple-system,BlinkMacSystemFont,Helvetica,Arial,sans-serif";
const DISPLAY_FONT = "'Space Grotesk',Figtree,'Segoe UI',Helvetica,Arial,sans-serif";

const IGNORE_NOTE = "If you didn't ask for this, you can ignore this email and nothing will change.";

export function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

function siteHost(siteUrl) {
  try {
    return new URL(siteUrl).host;
  } catch {
    return "";
  }
}

function sentByLine(instanceName, siteUrl) {
  const host = siteHost(siteUrl);
  return host ? `Sent by ${instanceName} from ${host}` : `Sent by ${instanceName}`;
}

function paragraphs(body) {
  return [body].flat().filter(Boolean);
}

function renderText({ instanceName, siteUrl, heading, body, actionLabel, url, note }) {
  const link = url ? [`${actionLabel}:\n${url}`] : [];
  return [instanceName, heading, ...paragraphs(body), ...link, note, sentByLine(instanceName, siteUrl)].join("\n\n") + "\n";
}

function header(instanceName) {
  return `<tr><td style="padding:0 4px 20px">
<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr>
<td style="vertical-align:middle"><img src="cid:${ICON_CID}" width="36" height="36" alt="" style="display:block;border:0;border-radius:9px"></td>
<td style="vertical-align:middle;padding-left:12px;font-family:${DISPLAY_FONT};font-size:22px;line-height:28px;font-weight:700;color:${COLOR.heading}">${escapeHtml(instanceName)}</td>
</tr></table>
</td></tr>`;
}

function button(actionLabel, href) {
  return `<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr>
<td bgcolor="${COLOR.accent}" style="background:${COLOR.accent};border-radius:999px">
<a href="${href}" style="display:inline-block;padding:13px 24px;font-family:${TEXT_FONT};font-size:16px;line-height:20px;font-weight:700;color:${COLOR.accentInk};text-decoration:none;border-radius:999px">${escapeHtml(actionLabel)}</a>
</td>
</tr></table>`;
}

function paragraphHtml(text, isLast) {
  return `<p style="margin:0 0 ${isLast ? 0 : 16}px;font-family:${TEXT_FONT};font-size:16px;line-height:24px;color:${COLOR.text}">${escapeHtml(text)}</p>`;
}

function action(actionLabel, url) {
  const href = escapeHtml(url);
  return `<div style="height:28px;line-height:28px;font-size:0">&nbsp;</div>
${button(actionLabel, href)}
<p style="margin:28px 0 6px;font-family:${TEXT_FONT};font-size:13px;line-height:20px;color:${COLOR.muted}">If the button doesn't work, paste this link into your browser:</p>
<p style="margin:0;font-family:${TEXT_FONT};font-size:13px;line-height:20px;word-break:break-all"><a href="${href}" style="color:${COLOR.heading};text-decoration:underline">${href}</a></p>`;
}

function card({ heading, body, actionLabel, url }) {
  const texts = paragraphs(body);
  return `<tr><td bgcolor="${COLOR.panel}" style="background:${COLOR.panel};border:1px solid ${COLOR.line};border-radius:16px;padding:32px 28px">
<h1 style="margin:0 0 12px;font-family:${DISPLAY_FONT};font-size:24px;line-height:30px;font-weight:700;color:${COLOR.heading}">${escapeHtml(heading)}</h1>
${texts.map((text, i) => paragraphHtml(text, i === texts.length - 1)).join("\n")}
${url ? action(actionLabel, url) : ""}
</td></tr>`;
}

function footer(instanceName, siteUrl, note) {
  return `<tr><td style="padding:20px 4px 0;font-family:${TEXT_FONT};font-size:13px;line-height:20px;color:${COLOR.faint}">
${escapeHtml(note)}<br>${escapeHtml(sentByLine(instanceName, siteUrl))}
</td></tr>`;
}

function renderHtml({ instanceName, siteUrl, subject, heading, body, actionLabel, url, note }) {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="dark">
<meta name="supported-color-schemes" content="dark">
<title>${escapeHtml(subject)}</title>
</head>
<body style="margin:0;padding:0;background:${COLOR.ground}">
<div style="display:none;max-height:0;overflow:hidden">${escapeHtml(paragraphs(body)[0] ?? heading)}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="${COLOR.ground}" style="background:${COLOR.ground}">
<tr><td align="center" style="padding:40px 16px">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="max-width:480px">
${header(instanceName)}
${card({ heading, body, actionLabel, url })}
${footer(instanceName, siteUrl, note)}
</table>
</td></tr>
</table>
</body>
</html>
`;
}

export function renderEmail(message) {
  const filled = { subject: message.heading, note: IGNORE_NOTE, ...message };
  return { text: renderText(filled), html: renderHtml(filled) };
}

export function createMailer(config) {
  const transport = config.smtp ? nodemailer.createTransport(config.smtp.transport) : null;

  return async function send({ to, ...message }) {
    if (!transport) {
      console.warn(`[mail] SMTP not configured, dropped one "${message.subject}" email`);
      return;
    }
    const { text, html } = renderEmail({ instanceName: config.instanceName, siteUrl: config.baseURL, ...message });
    return transport.sendMail({
      from: config.smtp.from,
      ...(config.smtp.replyTo ? { replyTo: config.smtp.replyTo } : {}),
      to,
      subject: message.subject,
      text,
      html,
      attachments: [{ filename: "icon.png", content: ICON, cid: ICON_CID, contentDisposition: "inline" }],
      headers: { "Auto-Submitted": "auto-generated", "X-Auto-Response-Suppress": "All" },
    });
  };
}
