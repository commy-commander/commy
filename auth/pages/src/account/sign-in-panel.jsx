import { useEffect, useRef, useState } from "react";
import { AuthError, authPost } from "../lib/auth-client.js";
import { passkeyErrorMessage, passkeysWork, signInWithPasskey, signInWithPasskeyAutofill } from "../lib/passkey.js";
import { signUpHref, twoFactorHref } from "../lib/safe-next.js";
import { useAction } from "../lib/use-action.js";
import { useSite } from "../site.jsx";
import Turnstile from "./turnstile.jsx";

export const SOCIAL_LABELS = {
  apple: "Apple",
  google: "Google",
  twitch: "Twitch",
};

export const SOCIAL_ICONS = {
  apple: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path
        fill="currentColor"
        d="M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701"
      />
    </svg>
  ),
  google: (
    <svg viewBox="0 0 18 18" aria-hidden="true">
      <path fill="currentColor" d="M17.64 9.2045c0-.6381-.0573-1.2518-.1636-1.8409H9v3.4814h4.8436c-.2086 1.125-.8427 2.0782-1.7959 2.7164v2.2581h2.9087c1.7018-1.5668 2.6836-3.874 2.6836-6.615z" />
      <path fill="currentColor" d="M9 18c2.43 0 4.4673-.8059 5.9564-2.1805l-2.9087-2.2581c-.8059.54-1.8368.859-3.0477.859-2.344 0-4.3282-1.5831-5.036-3.7104H.9574v2.3318C2.4382 15.9832 5.4818 18 9 18z" />
      <path fill="currentColor" d="M3.964 10.71c-.18-.54-.2822-1.1168-.2822-1.71s.1023-1.17.2823-1.71V4.9582H.9573A8.9965 8.9965 0 0 0 0 9c0 1.4523.3477 2.8268.9573 4.0418L3.964 10.71z" />
      <path fill="currentColor" d="M9 3.5795c1.3214 0 2.5077.4541 3.4405 1.346l2.5813-2.5814C13.4632.8918 11.426 0 9 0 5.4818 0 2.4382 2.0168.9573 4.9582L3.964 7.29C4.6718 5.1627 6.6559 3.5795 9 3.5795z" />
    </svg>
  ),
  twitch: (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path
        fill="currentColor"
        d="M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.43l-3 3v-3H6.857V1.714h13.714Z"
      />
    </svg>
  ),
  link: (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="5" width="18" height="14" rx="3" />
      <path d="M4 7.5l8 6 8-6" />
    </svg>
  ),
  passkey: (
    <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="9" cy="8" r="4" />
      <path d="M2.5 20a6.5 6.5 0 0 1 10.6-5" />
      <circle cx="17.5" cy="15.5" r="2.5" />
      <path d="M17.5 18v3.5M17.5 20h1.8" />
    </svg>
  ),
};

const SENT_COPY = {
  verify: {
    title: "Check your email",
    body: "Open the link we sent to finish creating your account.",
  },
  magic: {
    title: "Check your email",
    body: "Open the link we sent to sign in. It works once and expires soon.",
  },
  reset: {
    title: "Check your email",
    body: "If an account uses that address, a reset link is on its way. It expires in 1 hour.",
  },
};

const ADULT_REQUIRED = "Tick the box to confirm you are 18 or older.";
const ADULT_CONFIRMED = { adultConfirmed: true };

const SUBMIT_LABELS = {
  signin: "Sign in",
  signup: "Create account",
  magic: "Email me a link",
  forgot: "Email me a reset link",
};

const OTHER_WAYS = ["google", "twitch"];

function OtherWays({ providers, busy, onPick }) {
  const icons = OTHER_WAYS.filter((p) => providers[p]);
  if (!providers.apple && icons.length === 0) return null;
  return (
    <div className="other-ways">
      <div className="divider">or</div>
      {providers.apple && (
        <button type="button" className="btn-apple" onClick={() => onPick("apple")} disabled={busy}>
          {SOCIAL_ICONS.apple}
          <span>Continue with Apple</span>
        </button>
      )}
      {icons.length > 0 && (
        <div className="social-icons">
          {icons.map((p) => (
            <button
              key={p}
              type="button"
              className="btn-social-icon"
              aria-label={`Continue with ${SOCIAL_LABELS[p]}`}
              title={`Continue with ${SOCIAL_LABELS[p]}`}
              onClick={() => onPick(p)}
              disabled={busy}
            >
              {SOCIAL_ICONS[p]}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function usePasskeyAutofill(active, callbackURL, setError) {
  useEffect(() => {
    if (!active) return undefined;
    let live = true;
    signInWithPasskeyAutofill()
      .then((signedIn) => {
        if (signedIn && live) window.location.href = callbackURL;
      })
      .catch((err) => {
        if (live && err instanceof AuthError) setError(err.message);
      });
    return () => {
      live = false;
    };
  }, [active, callbackURL]);
}

export default function SignInPanel({
  providers,
  callbackURL,
  initialMode = "signin",
}) {
  const { links } = useSite();
  const [mode, setMode] = useState(initialMode);
  const [step, setStep] = useState("email");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [sent, setSent] = useState(null);
  const { busy, error, setError, run: runAction } = useAction();
  const [notice, setNotice] = useState("");
  const [adult, setAdult] = useState(false);
  const creating = initialMode === "signup";
  const bot = useRef(null);
  const passwordInput = useRef(null);
  const captcha = () => bot.current?.headers() ?? {};
  const requireCheck = () => {
    const unfinished = bot.current?.unfinished();
    if (unfinished) throw new Error(unfinished);
  };
  const asksEmailFirst = mode === "signin" && step === "email";
  const passkeyHere = providers.passkey && mode === "signin";
  const usesPassword = mode === "signin" || mode === "signup";
  const asksAdult = creating && providers.adultsOnly && mode !== "forgot";
  const requireAdult = () => {
    if (asksAdult && !adult) throw new Error(ADULT_REQUIRED);
  };

  usePasskeyAutofill(passkeyHere && step === "email", callbackURL, setError);

  useEffect(() => {
    if (mode === "signin" && step === "password") passwordInput.current?.focus();
  }, [mode, step]);

  function run(action) {
    setNotice("");
    return runAction(action);
  }

  function switchTo(next) {
    setError("");
    setNotice("");
    setMode(next);
  }

  function changeEmail() {
    setError("");
    setNotice("");
    setPassword("");
    setMode("signin");
    setStep("email");
  }

  const social = (provider) =>
    run(async () => {
      requireAdult();
      const res = await authPost("/sign-in/social", {
        provider,
        callbackURL,
        errorCallbackURL: "/account",
        ...(asksAdult ? { additionalData: ADULT_CONFIRMED } : {}),
      });
      if (res.url) window.location.href = res.url;
    });

  const passkey = () =>
    run(async () => {
      try {
        if (!passkeysWork()) throw new Error("This browser cannot use passkeys.");
        await signInWithPasskey();
      } catch (err) {
        throw err instanceof AuthError ? err : new Error(passkeyErrorMessage(err, "Could not sign in with a passkey."));
      }
      window.location.href = callbackURL;
    });

  const resend = () =>
    run(async () => {
      requireCheck();
      await authPost("/send-verification-email", { email, callbackURL }, captcha());
      setNotice(`Sent another link to ${email}. Check your spam folder if it does not arrive.`);
    });

  const submitWhenChecked = (event) => {
    event.preventDefault();
    if (asksEmailFirst && !password) {
      setError("");
      setNotice("");
      setStep("password");
      return;
    }
    const unfinished = asksAdult && !adult ? ADULT_REQUIRED : bot.current?.unfinished();
    if (!unfinished) {
      submit(new FormData(event.currentTarget));
      return;
    }
    setNotice("");
    setError(unfinished);
  };

  const submit = (form) =>
    run(async () => {
      requireAdult();
      if (mode === "signup") {
        const res = await authPost(
          "/sign-up/email",
          {
            name: String(form.get("name") ?? "").trim() || email.split("@")[0],
            email,
            password,
            callbackURL,
            ...(asksAdult ? ADULT_CONFIRMED : {}),
          },
          captcha(),
        );
        if (!res.token) {
          setSent("verify");
          return;
        }
      } else if (mode === "signin") {
        try {
          const res = await authPost(
            "/sign-in/email",
            { email, password, callbackURL },
            captcha(),
          );
          if (res.twoFactorRedirect) {
            window.location.href = twoFactorHref(callbackURL);
            return;
          }
        } catch (err) {
          if (err instanceof AuthError && err.code === "EMAIL_NOT_VERIFIED") {
            setSent("verify");
            return;
          }
          setStep("password");
          throw err;
        }
      } else if (mode === "magic") {
        await authPost(
          "/sign-in/magic-link",
          { email, callbackURL, ...(asksAdult ? { metadata: ADULT_CONFIRMED } : {}) },
          captcha(),
        );
        setSent("magic");
        return;
      } else {
        await authPost(
          "/request-password-reset",
          { email, redirectTo: `${window.location.origin}/account/reset-password` },
          captcha(),
        );
        setSent("reset");
        return;
      }
      window.location.href = callbackURL;
    });

  const messages = (
    <>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {notice && (
        <p className="form-notice" role="status">
          {notice}
        </p>
      )}
    </>
  );

  if (sent) {
    const copy = SENT_COPY[sent];
    return (
      <div className="account-card">
        <h2>{copy.title}</h2>
        <p>
          We emailed <strong>{email}</strong>. {copy.body}
        </p>
        <div className="sent-actions">
          {sent === "verify" && (
            <button type="button" className="btn btn-ghost" onClick={resend} disabled={busy}>
              Send the link again
            </button>
          )}
          <button
            type="button"
            className="link-button"
            onClick={() => {
              setSent(null);
              setError("");
              setNotice("");
            }}
          >
            Use a different address
          </button>
        </div>
        <p className="muted small">It can take a minute. Check your spam folder if it does not arrive.</p>
        {messages}
        <Turnstile ref={bot} siteKey={providers.turnstileSiteKey} />
      </div>
    );
  }

  const heading = { forgot: "Reset your password", magic: "Get a sign-in link" }[mode] ?? null;
  const showsEmailField = !(mode === "signin" && step === "password");
  const passwordTucked = asksEmailFirst;
  const submitLabel = asksEmailFirst && !password ? "Continue" : SUBMIT_LABELS[mode];

  return (
    <div className="account-card">
      {heading && <h2>{heading}</h2>}
      <form className="signin-form" onSubmit={submitWhenChecked}>
        {showsEmailField ? (
          <label>
            <span>Email</span>
            <input
              type="email"
              name="email"
              placeholder="you@example.com"
              autoComplete={passkeyHere ? "username webauthn" : "username"}
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
            />
          </label>
        ) : (
          <div className="signin-as">
            <input className="tucked" type="email" autoComplete="username" value={email} readOnly tabIndex={-1} aria-hidden="true" />
            <span className="signin-as-email">{email}</span>
            <button type="button" className="link-button" onClick={changeEmail}>
              Change
            </button>
          </div>
        )}
        {usesPassword && (
          <label
            key={passwordTucked ? "tucked" : "shown"}
            className={passwordTucked ? "tucked" : undefined}
            aria-hidden={passwordTucked || undefined}
          >
            <span>Password</span>
            <input
              ref={passwordInput}
              type="password"
              name="password"
              autoComplete={mode === "signup" ? "new-password" : "current-password"}
              minLength={8}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              tabIndex={passwordTucked ? -1 : undefined}
              required={!passwordTucked}
            />
            {mode === "signup" && <small className="field-hint">At least 8 characters.</small>}
          </label>
        )}
        {mode === "signup" && (
          <label>
            <span>Display name (optional)</span>
            <input type="text" name="name" autoComplete="nickname" maxLength={64} />
          </label>
        )}
        {asksAdult && (
          <label className="check-row adult-check">
            <input
              type="checkbox"
              checked={adult}
              aria-invalid={error === ADULT_REQUIRED || undefined}
              onChange={(e) => {
                setAdult(e.target.checked);
                if (e.target.checked && error === ADULT_REQUIRED) setError("");
              }}
            />
            <span>
              I am 18 or older{links.terms && (
                <>
                  {" "}and I agree to the <a href={links.terms}>Terms of Service</a>
                </>
              )}.
            </span>
          </label>
        )}
        <Turnstile ref={bot} siteKey={providers.turnstileSiteKey} />
        <button type="submit" className="btn btn-primary btn-block" disabled={busy}>
          {submitLabel}
        </button>
      </form>

      <div className="signin-links">
        {mode === "signin" && step === "password" && passkeyHere && (
          <button type="button" className="link-button" onClick={passkey} disabled={busy}>
            Sign in with a passkey
          </button>
        )}
        {mode === "signin" && step === "password" && providers.magicLink && (
          <button type="button" className="link-button" onClick={() => switchTo("magic")}>
            Email me a sign-in link
          </button>
        )}
        {mode === "signin" && step === "password" && providers.passwordReset && (
          <button type="button" className="link-button" onClick={() => switchTo("forgot")}>
            Forgot password?
          </button>
        )}
        {(mode === "forgot" || mode === "magic") && (
          <button type="button" className="link-button" onClick={() => switchTo("signin")}>
            Back to sign in
          </button>
        )}
      </div>

      {messages}

      {usesPassword && <OtherWays providers={providers} busy={busy} onPick={social} />}

      {usesPassword && (
        <p className="muted small signin-switch">
          {mode === "signin" ? (
            <>
              New here? <a href={signUpHref(callbackURL)}>Create an account</a>
            </>
          ) : (
            <>
              Already have an account? <a href={callbackURL}>Log in</a>
            </>
          )}
        </p>
      )}
      {mode === "signup" && links.privacy && (
        <p className="muted small">
          By creating an account you agree to the <a href={links.privacy}>privacy policy</a>.
        </p>
      )}
    </div>
  );
}
