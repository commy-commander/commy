function streamOpenAction({ userId, meId, sharing, watchingUser }) {
  if (userId && userId === meId && sharing) return "preview";
  if (userId && userId === watchingUser) return "already";
  return "watch";
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { streamOpenAction };
}
