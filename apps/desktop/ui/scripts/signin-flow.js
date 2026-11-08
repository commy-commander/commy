const SIGNIN_STEPS = ["form", "code", "new", "access"];
const CAPTCHA_MESSAGE = "commy-captcha";

function inAppSignIn(options, mobile) {
  const password = Boolean(mobile && options && options.password);
  const apple = Boolean(mobile && options && options.apple);
  return {
    form: password || apple,
    password,
    apple,
    captchaUrl: password && options.captcha_url ? options.captcha_url : null,
  };
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
  module.exports = { SIGNIN_STEPS, inAppSignIn, signInStepView, readCaptchaMessage };
}
