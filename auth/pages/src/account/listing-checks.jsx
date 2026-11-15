import { CHECK_NAMES } from "../lib/listing.js";

export default function ListingChecks({ checks }) {
  return (
    <ul className="plain-list listing-checks">
      {checks.map((check) => (
        <li key={check.id} className={check.ok ? "check-pass" : "check-fail"}>
          <strong>
            {check.ok ? "Passed" : "Failed"}: {CHECK_NAMES[check.id] ?? check.id}.
          </strong>{" "}
          {check.detail}
        </li>
      ))}
    </ul>
  );
}
