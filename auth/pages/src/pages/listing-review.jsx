import { useState } from "react";
import ListingChecks from "../account/listing-checks.jsx";
import { authPost } from "../lib/auth-client.js";
import { useAction } from "../lib/use-action.js";

const KIND_NAMES = { home: "accounts", sync: "communities" };

function Row({ label, value }) {
  return (
    <li>
      <strong>{label}:</strong> {value || "not given"}
    </li>
  );
}

export default function ListingReviewPage({ token, application }) {
  const [decided, setDecided] = useState(null);
  const [note, setNote] = useState("");
  const { busy, error, run } = useAction();

  if (!application) {
    return (
      <main className="page">
        <h1>Review a listing</h1>
        <p className="form-error" role="alert">
          That review link has expired, is not valid, or the application was already reviewed.
        </p>
      </main>
    );
  }

  const decide = (decision) =>
    run(async () => {
      const answer = await authPost("/commy/listing/decide", { token, decision, note: note.trim() || undefined });
      setDecided(answer.status);
    });

  return (
    <main className="page">
      <h1>Review {application.name}</h1>
      <div className="account-card">
        <h2>Application</h2>
        <ul className="plain-list">
          <Row label="Address" value={application.origin} />
          <Row label="Hosts" value={application.kinds.map((kind) => KIND_NAMES[kind] ?? kind).join(" and ")} />
          <Row label="Description" value={application.about} />
          <Row label="Abuse contact" value={application.contact} />
          <Row label="Operator" value={application.operatorEmail} />
          <Row label="Emergency admin" value={application.emergencyEmail} />
          <Row label="Terms" value={application.terms} />
          <Row label="Privacy" value={application.privacy} />
          <Row label="Applied" value={new Date(application.appliedAt).toISOString().slice(0, 10)} />
        </ul>
      </div>
      <div className="account-card">
        <h2>Checks</h2>
        <ListingChecks checks={application.checks} />
      </div>
      <div className="account-card">
        <h2>Decision</h2>
        {decided ? (
          <p className="form-notice" role="status">
            {decided === "listed" ? "Listed. The apps pick it up within the hour." : "Rejected. The operator was told."}
          </p>
        ) : (
          <div className="signin-form">
            <label>
              <span>Note to the operator</span>
              <input value={note} maxLength={500} onInput={(event) => setNote(event.currentTarget.value)} />
            </label>
            <div className="form-actions">
              <button type="button" className="btn btn-primary" disabled={busy} onClick={() => decide("approve")}>
                Approve and list
              </button>
              <button type="button" className="btn btn-danger" disabled={busy} onClick={() => decide("reject")}>
                Reject
              </button>
            </div>
            {error && (
              <p className="form-error" role="alert">
                {error}
              </p>
            )}
          </div>
        )}
      </div>
    </main>
  );
}
