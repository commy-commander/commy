import { useState } from "react";

export function errorMessage(err) {
  return err instanceof Error ? err.message : String(err);
}

export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function run(action) {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return { busy, error, setError, run };
}
