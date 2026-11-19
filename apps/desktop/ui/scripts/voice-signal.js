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

function linkIsRelayed(link) {
  return link.path === "relay";
}

function linkIsDirect(link) {
  return link.path === "direct" || link.path === "mixed";
}

function linkPathChip(path) {
  if (path === "relay") return { kind: "relay", label: t("ui.relayed") };
  if (path === "direct" || path === "mixed") return { kind: "direct", label: t("ui.direct") };
  return { kind: "none", label: t("ui.connecting") };
}

function linkPathText(path) {
  if (path === "direct") return t("ui.connected-directly");
  if (path === "relay") return t("ui.connected-through-a-relay");
  if (path === "mixed") return t("ui.connected-directly-relay-on-standby");
  return t("ui.connecting");
}

function ownPath(links) {
  const relayed = links.filter(linkIsRelayed).length;
  const direct = links.filter(linkIsDirect).length;
  if (!relayed && !direct) return { kind: "none", label: t("ui.waiting"), text: t("ui.nobody-else-yet") };
  if (!relayed) return { kind: "direct", label: t("ui.direct"), text: direct === 1 ? t("ui.direct") : t("ui.all-direct", { direct }) };
  if (!direct) return { kind: "relay", label: t("ui.relayed"), text: relayed === 1 ? t("ui.relayed") : t("ui.all-relayed", { relayed }) };
  return { kind: "mixed", label: t("ui.some-relayed"), text: t("ui.direct-relayed", { direct, relayed }) };
}

const OWN_BADGES = {
  good: { cls: "good", get label() { return t("ui.signal.good"); } },
  fair: { cls: "ok", get label() { return t("ui.signal.fair"); } },
  poor: { cls: "bad", get label() { return t("ui.signal.poor"); } },
  offline: { cls: "bad", get label() { return t("ui.offline-2"); } },
};

function ownBadge(quality) {
  return OWN_BADGES[quality] || OWN_BADGES.offline;
}

function ownNetworkText(direct) {
  if (direct === "open") return t("ui.direct-connections-work");
  if (direct === "limited") return t("ui.direct-with-some-people");
  if (direct === "blocked") return t("ui.relay-only");
  return t("ui.not-checked-yet");
}

function ownRelayText(me) {
  if (!me.relay_reachable) return t("ui.not-reachable");
  return Number.isFinite(me.relay_rtt_ms) ? `${Math.round(me.relay_rtt_ms)} ms` : t("ui.measuring");
}

function connEntries(me, links) {
  const others = links.map(link => ({ ...link, self: false, chip: linkPathChip(link.path) }));
  if (!me) return others;
  const path = ownPath(links);
  const you = { ...me, self: true, quality: ownBadge(me.quality).cls, chip: { kind: path.kind, label: path.label }, path_text: path.text };
  return [you, ...others];
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    SIGNAL_MIN_SAMPLE_PACKETS, linkQuality, createSignalTracker, worstLinkQuality,
    linkPathChip, linkPathText, ownPath, ownBadge, ownNetworkText, ownRelayText, connEntries,
  };
}
