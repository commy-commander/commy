const APP_ICON_MARK = "images/commy-star.svg";

const APP_ICON_PRESETS = [
  { id: "classic", get label() { return t("ui.periwinkle"); }, image: APP_ICON_MARK, tile: "linear-gradient(180deg, #7686ff, #5466f0)" },
  { id: "amber", get label() { return t("ui.amber"); }, image: APP_ICON_MARK, tile: "#f2b24c" },
  { id: "mint", get label() { return t("ui.mint"); }, image: APP_ICON_MARK, tile: "#8fdcbd" },
  { id: "ocean", get label() { return t("ui.sky"); }, image: APP_ICON_MARK, tile: "#c3d6ff" },
  { id: "violet", get label() { return t("ui.lilac"); }, image: APP_ICON_MARK, tile: "#dccdff" },
  { id: "mono", get label() { return t("ui.mono"); }, image: APP_ICON_MARK, tile: "#33363d" },
];

function appIconChoices(target, hasCustom) {
  const tiles = [];
  if (target === "tray") tiles.push({ id: "app", label: t("ui.same-as-app"), image: null, tile: null });
  for (const preset of APP_ICON_PRESETS) tiles.push(preset);
  tiles.push({ id: "custom", label: hasCustom ? t("ui.yours") : t("ui.your-own"), image: null, tile: null, custom: true });
  return tiles;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { APP_ICON_PRESETS, appIconChoices };
}
