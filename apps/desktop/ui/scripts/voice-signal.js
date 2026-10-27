const SIGNAL_MIN_SAMPLE_PACKETS = 50;

function linkQuality(rtt, loss) {
  if (rtt > 250 || loss > 5) return "bad";
  if (rtt > 120 || loss > 1) return "ok";
  return "good";
}

function hasPacketCounts(link) {
  return Number.isFinite(link.lost_packets) && Number.isFinite(link.sent_packets);
}

function countersRestarted(link, baseline) {
  return !baseline || link.sent_packets < baseline.sent || link.lost_packets < baseline.lost;
}

function lossSince(link, baseline) {
  const sent = link.sent_packets - baseline.sent;
  if (sent < SIGNAL_MIN_SAMPLE_PACKETS) return null;
  return Math.min(100, ((link.lost_packets - baseline.lost) * 100) / sent);
}

function createSignalTracker() {
  const baselines = new Map();
  const recentLoss = new Map();
  const remember = link => baselines.set(link.user, { lost: link.lost_packets, sent: link.sent_packets });

  const readLink = link => {
    if (!hasPacketCounts(link)) return link.loss_pct || 0;
    const baseline = baselines.get(link.user);
    if (countersRestarted(link, baseline)) {
      remember(link);
      return recentLoss.get(link.user) ?? 0;
    }
    const loss = lossSince(link, baseline);
    if (loss === null) return recentLoss.get(link.user) ?? 0;
    remember(link);
    recentLoss.set(link.user, loss);
    return loss;
  };

  return {
    read(links) {
      const live = new Set(links.map(link => link.user));
      for (const user of [...baselines.keys()]) {
        if (!live.has(user)) {
          baselines.delete(user);
          recentLoss.delete(user);
        }
      }
      return links.map(link => {
        const lossPct = readLink(link);
        return { ...link, loss_pct: lossPct, quality: linkQuality(link.rtt_ms, lossPct) };
      });
    },
    reset() {
      baselines.clear();
      recentLoss.clear();
    },
  };
}

function worstLinkQuality(links) {
  return links.reduce((worst, link) => {
    const q = link.quality;
    return q === "bad" || (q === "ok" && worst === "good") ? q : worst;
  }, "good");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SIGNAL_MIN_SAMPLE_PACKETS, linkQuality, createSignalTracker, worstLinkQuality };
}
