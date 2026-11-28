const DAY_KEY_CHOICES = [1, 7, 30, 90];

function dayKeysDaysLabel(days) {
  return days === 1 ? t("ui.day-keys-1-day") : t("ui.day-keys-n-days", { count: days });
}

function dayKeysStatus(view, when) {
  if (!view) return "";
  if (view.on) return t("ui.day-keys-on-since", { value: when(view.since) });
  if (view.chosen) return t("ui.day-keys-off-by-owner");
  if (!view.waiting) return t("ui.day-keys-turning-on");
  return t(view.waiting === 1 ? "ui.day-keys-waiting-one" : "ui.day-keys-waiting", { count: view.waiting });
}

function dayKeysWarnsOlderApps(view) {
  return Boolean(view && !view.on && view.waiting > 0 && view.can_turn_on);
}

function dayKeysControls(view) {
  const choices = view && Array.isArray(view.history_choices) && view.history_choices.length ? view.history_choices : DAY_KEY_CHOICES;
  return {
    on: Boolean(view && view.on),
    toggleDisabled: !view || (view.on ? !view.can_turn_off : !view.can_turn_on),
    historyDisabled: !view || !view.can_turn_on,
    choices,
    days: view && choices.includes(view.history_days) ? view.history_days : 30,
  };
}

function canSeeMessagePrivacy(server) {
  return Boolean(server && !server.group && server.day_keys && server.day_keys.can_turn_on);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { DAY_KEY_CHOICES, dayKeysDaysLabel, dayKeysStatus, dayKeysWarnsOlderApps, dayKeysControls, canSeeMessagePrivacy };
}
