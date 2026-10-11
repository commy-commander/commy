import nodemailer from "nodemailer";

export function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

export function renderEmail({ instanceName, heading, body, actionLabel, url }) {
  const text = `${heading}\n\n${body}\n\n${actionLabel}: ${url}\n\nIf you did not request this, you can ignore this email.`;
  const html = `<div style="font-family:system-ui,sans-serif;max-width:480px;margin:auto;padding:24px">
<h2 style="margin:0 0 12px">${escapeHtml(heading)}</h2>
<p>${escapeHtml(body)}</p>
<p><a href="${escapeHtml(url)}" style="display:inline-block;background:#ff4d4d;color:#fff;padding:10px 18px;border-radius:6px;text-decoration:none">${escapeHtml(actionLabel)}</a></p>
<p style="color:#666;font-size:13px">If you did not request this, you can ignore this email. Sent by ${escapeHtml(instanceName)}.</p>
</div>`;
  return { text, html };
}

// Without SMTP configured, mail is dropped with a log line that names no one.
export function createMailer(config) {
  const transport = config.smtp ? nodemailer.createTransport(config.smtp.url) : null;

  return async function send({ to, subject, heading, body, actionLabel, url }) {
    if (!transport) {
      // Never log the recipient or the link: either would leak account access
      console.warn(`[mail] SMTP_URL not set, dropped one "${subject}" email`);
      return;
    }
    const { text, html } = renderEmail({ instanceName: config.instanceName, heading, body, actionLabel, url });
    await transport.sendMail({ from: config.smtp.from, to, subject, text, html });
  };
}
