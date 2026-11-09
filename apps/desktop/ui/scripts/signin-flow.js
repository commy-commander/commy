const SIGNIN_STEPS = ["form", "code", "new", "access"];
const CAPTCHA_MESSAGE = "commy-captcha";

const SHEET_PROVIDERS = ["google", "twitch"];

function inAppSignIn(options, mobile) {
  const offered = name => Boolean(mobile && options && options[name]);
  const password = offered("password");
  const apple = offered("apple");
  const google = offered("google");
  const twitch = offered("twitch");
  const passkey = offered("passkey");
  const buttons = apple || google || twitch || passkey;
  return {
    form: password || buttons,
    password,
    apple,
    google,
    twitch,
    passkey,
    buttons,
    captchaUrl: password && options.captcha_url ? options.captcha_url : null,
  };
}

function inAppRetry(method, { adultConfirmed = false, accessCode = null } = {}) {
  if (method && method.kind === "sheet" && SHEET_PROVIDERS.includes(method.provider)) {
    return { command: "account_sign_in_sheet", args: { provider: method.provider, adultConfirmed, accessCode } };
  }
  return { command: "account_sign_in_apple", args: { create: true, adultConfirmed, accessCode } };
}

function signInStepView(step) {
  const views = { code: "code", new_account: "new", confirm_adult: "new", access_code: "access" };
  return views[step] || "form";
}

function readCaptchaMessage(event, captchaUrl) {
  if (!event || !captchaUrl) return null;
  let origin;
  try {
    origin = new URL(captchaUrl).origin;
  } catch {
    return null;
  }
  const data = event.data;
  if (event.origin !== origin || !data || data.type !== CAPTCHA_MESSAGE) return null;
  if (typeof data.token === "string" && data.token) return { token: data.token };
  if (data.expired) return { expired: true };
  if (data.failed) return { failed: true };
  return null;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SIGNIN_STEPS, SHEET_PROVIDERS, inAppSignIn, inAppRetry, signInStepView, readCaptchaMessage };
}
