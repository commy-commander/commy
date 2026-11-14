import { useState } from "react";
import { unlock } from "../lib/access.js";
import { submitForm } from "../lib/forms.js";
import { useAction } from "../lib/use-action.js";

export default function AccessGate({ next }) {
  const [opening, setOpening] = useState(false);
  const { busy, error, run } = useAction();

  const submit = (form) =>
    run(async () => {
      await unlock(String(form.get("code") ?? ""));
      setOpening(true);
      window.location.assign(next + window.location.hash);
    });

  return (
    <form className="signin-form access-form" onSubmit={submitForm(submit)}>
      <label>
        <span>Access code</span>
        <input
          type="text"
          name="code"
          placeholder="Access code"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          maxLength={128}
          required
        />
        <small className="field-hint">The code the person who invited you shared. It is not the code shown in the Commy app.</small>
      </label>
      <button type="submit" className="btn btn-primary" disabled={busy || opening}>
        Continue
      </button>
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}
