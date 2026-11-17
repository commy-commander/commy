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
    socialIcons: apple && (google || twitch),
    signUp: password && offered("sign_up"),
    captchaUrl: password && options.captcha_url ? options.captcha_url : null,
  };
}

const MODE_TEXT = {
  signin: { title: "signin.title", intro: "signin.intro", apple: "signin.apple", switchLink: "signin.create" },
  signup: { title: "signup.title", intro: "signup.intro", apple: "signup.apple", switchLink: "signup.have.account" },
};

function signInModeText(mode) {
  return MODE_TEXT[mode] || MODE_TEXT.signin;
}

function signInFormParts(view, stage, mode = "signin") {
  if (mode === "signup") return signUpFormParts(view, stage);
  const askPassword = Boolean(view.password && stage === "password");
  return {
    email: Boolean(view.password),
    password: askPassword,
    name: false,
    adult: false,
    inbox: false,
    socials: true,
    submitLabel: askPassword ? "signin.submit" : "signin.continue",
    passkeyLink: askPassword && Boolean(view.passkey),
    captchaUrl: askPassword ? view.captchaUrl : null,
  };
}

function signUpFormParts(view, stage) {
  const offered = Boolean(view.signUp);
  const details = offered && stage === "details";
  const inbox = offered && stage === "inbox";
  return {
    email: offered && !inbox,
    password: details,
    name: details,
    adult: details,
    inbox,
    socials: !details && !inbox,
    submitLabel: details ? "signup.submit" : "signin.continue",
    passkeyLink: false,
    captchaUrl: details || inbox ? view.captchaUrl : null,
  };
}

const RETURN_RETRY_MS = 15000;

function returnSignInDue({ stage, mode, triedAt = 0, now = 0, fromLink = false }) {
  if (mode !== "signup" || stage !== "inbox") return false;
  return fromLink || now - triedAt >= RETURN_RETRY_MS;
}

function showsOfficialBadge(choice, defaultHost) {
  return choice === "default" && (defaultHost || "commy.chat") === "commy.chat";
}

function looksLikeEmail(text) {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(String(text || "").trim());
}

function emailNextStep(view, filledPassword) {
  if (filledPassword || !view.passkey) return "password";
  return "passkey";
}

function passkeyFallsBackToPassword(result) {
  const step = result && result.step;
  return step === "canceled" || step === "failed";
}

function inAppRetry(method, { adultConfirmed = false, accessCode = null, captcha = null } = {}) {
  if (method && method.kind === "email") {
    const { email, password, name } = method;
    return { command: "account_sign_up_email", args: { email, password, name, adultConfirmed: true, captcha, accessCode } };
  }
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
  module.exports = {
    SIGNIN_STEPS,
    SHEET_PROVIDERS,
    inAppSignIn,
    signInModeText,
    returnSignInDue,
    signInFormParts,
    showsOfficialBadge,
    looksLikeEmail,
    emailNextStep,
    passkeyFallsBackToPassword,
    inAppRetry,
    signInStepView,
    readCaptchaMessage,
  };
}
