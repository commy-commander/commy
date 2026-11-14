import { verifyEmailOutcome } from "../lib/verify-email.js";
import { useSite } from "../site.jsx";

const ICONS = {
  confirmed: <path d="M5 12.5l4.5 4.5L19 7.5" />,
  expired: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 2" />
    </>
  ),
  invalid: <path d="M7 7l10 10M17 7L7 17" />,
};

export default function VerifyEmailPage({ result, next }) {
  const { name } = useSite();
  const outcome = verifyEmailOutcome(result, next, name);
  return (
    <main className="page">
      <div className="account-card verify-result" data-result={outcome.result}>
        <span className="verify-result-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
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
