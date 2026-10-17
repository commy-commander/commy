// The stable colour each person's avatar gets, shared by the main window
// and the in-game overlay so both draw the same face for the same person
// (see ui/tests/tint.test.mjs).

function personTint(userId) {
  let hash = 0;
  for (const ch of String(userId)) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  return `hsl(${Math.abs(hash) % 360} 38% 44%)`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { personTint };
}
