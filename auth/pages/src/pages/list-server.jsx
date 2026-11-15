import { useState } from "react";
import ListingChecks from "../account/listing-checks.jsx";
import { authPost } from "../lib/auth-client.js";
import { PLEDGE } from "../lib/listing.js";
import { useAction } from "../lib/use-action.js";
import { useSite } from "../site.jsx";

function Pledge() {
  return (
    <ol className="plain-list listing-pledge">
      {PLEDGE.map(([promise, detail]) => (
        <li key={promise}>
          <strong>{promise}.</strong> {detail}
        </li>
      ))}
    </ol>
  );
}

function Field({ label, name, type = "text", required = false, placeholder, hint, maxLength = 200 }) {
  return (
    <label>
      <span>{label}</span>
      <input name={name} type={type} required={required} placeholder={placeholder} maxLength={maxLength} autoComplete="off" />
      {hint && <small className="muted">{hint}</small>}
    </label>
  );
}

function ApplyForm() {
  const [result, setResult] = useState(null);
  const { busy, error, run } = useAction();

  const apply = (event) => {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const value = (name) => String(form.get(name) ?? "").trim() || undefined;
    run(async () => {
      setResult(null);
      const answer = await authPost("/commy/listing/apply", {
        origin: value("origin") ?? "",
        name: value("name"),
        about: value("about"),
        operatorEmail: value("operatorEmail") ?? "",
        emergencyEmail: value("emergencyEmail") ?? "",
        terms: value("terms"),
        privacy: value("privacy"),
        pledge: form.get("pledge") === "on",
      });
      setResult(answer);
    });
  };

  return (
    <form className="signin-form" onSubmit={apply}>
      <Field label="Server address" name="origin" required placeholder="https://chat.example.org" maxLength={300} />
      <Field label="Name in the list" name="name" placeholder="Example Chat" maxLength={60} hint="Leave empty to use the name your server publishes." />
      <Field label="One-line description" name="about" maxLength={140} placeholder="A server for the Example club" />
      <Field label="Your email" name="operatorEmail" type="email" required maxLength={254} hint="We write here about the listing. It is never shown in the apps." />
      <Field label="Emergency admin's email" name="emergencyEmail" type="email" required maxLength={254} />
      <Field label="Terms page" name="terms" placeholder="https://example.org/terms" maxLength={500} hint="Only needed if your server hosts communities but not accounts." />
      <Field label="Privacy page" name="privacy" placeholder="https://example.org/privacy" maxLength={500} />
      <label className="check-row">
        <input type="checkbox" name="pledge" required />
        <span>I run this server and keep every promise in the pledge above.</span>
      </label>
      <div className="form-actions">
        <button type="submit" className="btn btn-primary" disabled={busy}>
          {busy ? "Checking your server…" : "Check and apply"}
        </button>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {result && <ApplyResult result={result} />}
    </form>
  );
}

function ApplyResult({ result }) {
  return (
    <div role="status">
      <p className={result.passed ? "form-notice" : "form-error"}>
        {result.passed
          ? `Every check passed. We emailed ${result.confirmTo} a link to confirm the abuse contact; after that, a person reviews your application.`
          : "Some checks failed. Fix them and apply again."}
      </p>
      <ListingChecks checks={result.checks ?? []} />
    </div>
  );
}

function ConfirmContact({ token }) {
  const [done, setDone] = useState(false);
  const { busy, error, run } = useAction();
  const confirm = () =>
    run(async () => {
      await authPost("/commy/listing/confirm", { token });
      setDone(true);
    });
  if (done) {
    return (
      <p className="form-notice" role="status">
        Thanks, the contact is confirmed. A person reviews the application next, and we email the operator with the result.
      </p>
    );
  }
  return (
    <div className="account-section">
      <p>Confirm that a person reads this address and answers abuse reports for the server.</p>
      <div className="form-actions">
        <button type="button" className="btn btn-primary" disabled={busy} onClick={confirm}>
          Confirm the contact
        </button>
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

export default function ListServerPage({ confirm }) {
  const { name } = useSite();
  return (
    <main className="page">
      <h1>List your server on {name}</h1>
      <p className="muted">
        Listed servers show up when people pick where to sign in, and joining a community on one needs no extra step. A
        server that is not listed still works: the apps ask people to confirm they trust it first.
      </p>
      {confirm ? (
        <div className="account-card">
          <h2>Confirm the abuse contact</h2>
          <ConfirmContact token={confirm} />
        </div>
      ) : (
        <>
          <div className="account-card">
            <h2>The pledge</h2>
            <Pledge />
          </div>
          <div className="account-card">
            <h2>Apply</h2>
            <p className="muted">
              We check your server straight away and every month after it is listed: its address, certificate, discovery
              document, version, abuse contact, Terms and privacy pages, and the 18+ check. A server that fails for 7 days
              in a row is taken off the list.
            </p>
            <ApplyForm />
          </div>
        </>
      )}
    </main>
  );
}
