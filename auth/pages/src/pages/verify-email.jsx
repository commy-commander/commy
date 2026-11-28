import { verifyEmailOutcome } from "../lib/verify-email.js";
import { useSite } from "../site.jsx";

const ICONS = {
  confirmed: <path d="m8.5 16.59-3.8-3.8a1 1 0 0 0-1.4 1.42l4.5 4.5a1 1 0 0 0 1.4 0l11-11a1 1 0 0 0-1.4-1.42L8.5 16.6Z" />,
  expired: <path d="M15.25 13.5h-4a.75.75 0 0 1-.75-.75v-6a.75.75 0 0 1 1.5 0V12h3.25a.75.75 0 0 1 0 1.5ZM12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20Z" />,
  invalid: <path d="m4.21 4.39.08-.1a1 1 0 0 1 1.32-.08l.1.08L12 10.6l6.3-6.3a1 1 0 1 1 1.4 1.42L13.42 12l6.3 6.3a1 1 0 0 1 .08 1.31l-.08.1a1 1 0 0 1-1.32.08l-.1-.08L12 13.4l-6.3 6.3a1 1 0 0 1-1.4-1.42L10.58 12l-6.3-6.3a1 1 0 0 1-.08-1.31l.08-.1-.08.1Z" />,
};

export default function VerifyEmailPage({ result, next }) {
  const { name } = useSite();
  const outcome = verifyEmailOutcome(result, next, name);
  return (
    <main className="page">
      <div className="account-card verify-result" data-result={outcome.result}>
        <span className="verify-result-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="currentColor">
            {ICONS[outcome.result]}
          </svg>
        </span>
        <h1>{outcome.title}</h1>
        <p>{outcome.body}</p>
        <div className="sent-actions">
          <a href={outcome.action.href} className="btn btn-primary">
            {outcome.action.label}
          </a>
        </div>
      </div>
    </main>
  );
}
