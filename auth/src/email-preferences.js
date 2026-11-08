import * as z from "zod";
import { createAuthEndpoint } from "better-auth/api";
import { COLOR, DISPLAY_FONT, TEXT_FONT, escapeHtml } from "./mailer.js";
import { PREFERENCES_PATH, emailLinkSigner } from "./email-links.js";

export const MARKETING_OFF_FIELD = "emailMarketingOff";

const PREFERENCE_BODY_TYPES = ["application/x-www-form-urlencoded", "multipart/form-data", "application/json", "text/plain"];

const PAGE_HEADERS = {
  "content-type": "text/html; charset=utf-8",
  "cache-control": "no-store",
  "referrer-policy": "no-referrer",
  "x-robots-tag": "noindex",
  "content-security-policy": "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
};

const ACCOUNT_EMAILS =
  "Always on. Sign-in links, password resets, codes, and alerts when your password, passkeys or sign-in methods change.";

function preferencesView(address, marketingOff, saved) {
  return {
    heading: "Manage your emails",
    status: saved ? (marketingOff ? "Saved. You're unsubscribed from marketing emails." : "Saved. Marketing emails are back on.") : null,
    intro: `Email settings for ${address}.`,
    sections: [
      { title: "Account emails", text: ACCOUNT_EMAILS },
      {
        title: "Marketing emails",
        text: marketingOff ? "Off. You won't get news or announcements." : "On. News and announcements.",
        choice: marketingOff ? "on" : "off",
        actionLabel: marketingOff ? "Subscribe again" : "Unsubscribe",
      },
    ],
  };
}

const INVALID_VIEW = {
  heading: "This link doesn't work",
  intro: "Open the link at the bottom of your latest email again.",
  sections: [],
};

function formHtml(formAction, choice, actionLabel) {
  return `<form method="post" action="${escapeHtml(formAction)}" style="margin:16px 0 0">
<input type="hidden" name="choice" value="${choice}">
<button type="submit" style="cursor:pointer;border:0;background:${COLOR.accent};color:${COLOR.accentInk};border-radius:999px;padding:13px 24px;font-family:${TEXT_FONT};font-size:16px;line-height:20px;font-weight:700">${escapeHtml(actionLabel)}</button>
</form>`;
}

function textHtml(text, color = COLOR.text) {
  return `<p style="margin:0;font-family:${TEXT_FONT};font-size:16px;line-height:24px;color:${color}">${escapeHtml(text)}</p>`;
}

function statusHtml(status) {
  if (!status) return "";
  return `<p role="status" style="margin:0 0 20px;padding:12px 16px;border-radius:12px;background:${COLOR.line};font-family:${TEXT_FONT};font-size:15px;line-height:22px;font-weight:700;color:${COLOR.heading}">${escapeHtml(status)}</p>`;
}

function sectionHtml(section, formAction) {
  return `<div style="margin:24px 0 0;padding:24px 0 0;border-top:1px solid ${COLOR.line}">
<h2 style="margin:0 0 6px;font-family:${DISPLAY_FONT};font-size:18px;line-height:24px;font-weight:700;color:${COLOR.heading}">${escapeHtml(section.title)}</h2>
${textHtml(section.text, COLOR.muted)}
${section.choice ? formHtml(formAction, section.choice, section.actionLabel) : ""}
</div>`;
}

function pageHtml(instanceName, view, formAction) {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta name="color-scheme" content="dark">
<title>${escapeHtml(`${view.heading} · ${instanceName}`)}</title>
</head>
<body style="margin:0;min-height:100vh;background:${COLOR.ground};display:flex;align-items:center;justify-content:center;padding:40px 16px;box-sizing:border-box">
<main style="width:100%;max-width:480px">
<p style="margin:0 0 20px 4px;font-family:${DISPLAY_FONT};font-size:22px;line-height:28px;font-weight:700;color:${COLOR.heading}">${escapeHtml(instanceName)}</p>
<section style="background:${COLOR.panel};border:1px solid ${COLOR.line};border-radius:16px;padding:32px 28px">
${statusHtml(view.status)}
<h1 style="margin:0 0 12px;font-family:${DISPLAY_FONT};font-size:24px;line-height:30px;font-weight:700;color:${COLOR.heading}">${escapeHtml(view.heading)}</h1>
${textHtml(view.intro)}
${view.sections.map((section) => sectionHtml(section, formAction)).join("\n")}
</section>
</main>
</body>
</html>
`;
}

function requestMethod(ctx) {
  return (ctx.request?.method ?? ctx.method ?? "GET").toUpperCase();
}

function chosenMarketingOff(body) {
  return body?.choice !== "on";
}

export const commyEmailPreferences = ({ secret, instanceName, basePath }) => {
  const signer = emailLinkSigner(secret);
  const page = (status, view, formAction) => new Response(pageHtml(instanceName, view, formAction), { status, headers: PAGE_HEADERS });

  return {
    id: "commy-email-preferences",
    schema: {
      user: {
        fields: {
          [MARKETING_OFF_FIELD]: { type: "boolean", required: false, defaultValue: false, input: false, returned: false },
        },
      },
    },
    endpoints: {
      commyEmailPreferences: createAuthEndpoint(
        PREFERENCES_PATH,
        {
          method: ["GET", "POST"],
          query: z.object({ a: z.string().max(512).optional(), t: z.string().max(128).optional() }),
          body: z.any().optional(),
          metadata: { isAction: false, allowedMediaTypes: PREFERENCE_BODY_TYPES },
        },
        async (ctx) => {
          const address = signer.verify(ctx.query?.a, ctx.query?.t);
          if (!address) return page(400, INVALID_VIEW);
          const formAction = `${basePath}${PREFERENCES_PATH}?${signer.linkQuery(address)}`;
          const internal = ctx.context.internalAdapter;
          const found = await internal.findUserByEmail(address);

          if (requestMethod(ctx) !== "POST") {
            return page(200, preferencesView(address, Boolean(found?.user?.[MARKETING_OFF_FIELD]), false), formAction);
          }

          const off = chosenMarketingOff(ctx.body);
          if (found?.user) await internal.updateUser(found.user.id, { [MARKETING_OFF_FIELD]: off });
          return page(200, preferencesView(address, off, true), formAction);
        },
      ),
    },
  };
};
