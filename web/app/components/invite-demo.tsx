"use client";

import { useId, useState } from "react";

// Invite codes in Commy are eight characters from this alphabet. The demo derives a
// stable one from the name so the link on the left stays put while you type.
const ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";

export function demoCode(name: string): string {
  let h = 2166136261;
  for (const ch of name) {
    h ^= ch.charCodeAt(0);
    h = Math.imul(h, 16777619) >>> 0;
  }
  let out = "";
  for (let i = 0; i < 8; i++) {
    out += ALPHABET[h % ALPHABET.length];
    h = (Math.imul(h, 1103515245) + 12345) >>> 0;
  }
  return out;
}

function expiry(): string {
  const d = new Date();
  d.setDate(d.getDate() + 7);
  return d.toLocaleDateString("en-US", { month: "long", day: "numeric" });
}

// The one live thing on the page: name a server, read the invite link the app would
// hand out, and see the join prompt your friend gets, drawn the way the app draws it.
export default function InviteDemo() {
  const [name, setName] = useState("Pixel Pioneers");
  const id = useId();
  const clean = name.trim() || "Pixel Pioneers";
  const code = demoCode(clean.toLowerCase());

  return (
    <div className="invite-demo">
      <div className="invite-in">
        <label htmlFor={id}>Name your server</label>
        <input
          id={id}
          type="text"
          value={name}
          maxLength={40}
          autoComplete="off"
          spellCheck={false}
          onChange={(e) => setName(e.target.value)}
        />
        <p className="invite-link" aria-live="polite">
          https://commy.chat/i/<strong>{code}</strong>
        </p>
        <p className="invite-note">
          Links expire on their own, you can revoke them any time, and they open the app directly.
        </p>
      </div>
      <div className="invite-out">
        <div className="ui-dialog" data-ui role="img" aria-label={`The Commy join prompt for ${clean}`}>
          <p className="ui-title">Join {clean}?</p>
          <p className="ui-sub">Invite from commy.chat, expires {expiry()}</p>
          <div className="ui-actions">
            <span className="ui-btn">Cancel</span>
            <span className="ui-btn ui-btn-primary">Join Server</span>
          </div>
        </div>
      </div>
    </div>
  );
}
