const APP_ICON_MARK = "images/commy_v3_noback.png";

const APP_ICON_PRESETS = [
  { id: "classic", label: "Classic", image: APP_ICON_MARK, tile: "#ef443c" },
  { id: "amber", label: "Amber", image: APP_ICON_MARK, tile: "#e08a1e" },
  { id: "mint", label: "Mint", image: APP_ICON_MARK, tile: "#1f9e6e" },
  { id: "ocean", label: "Ocean", image: APP_ICON_MARK, tile: "#2a6fd1" },
  { id: "violet", label: "Violet", image: APP_ICON_MARK, tile: "#7448d4" },
  { id: "mono", label: "Mono", image: APP_ICON_MARK, tile: "#33363d" },
];

function appIconChoices(target, hasCustom) {
  const tiles = [];
  if (target === "tray") tiles.push({ id: "app", label: "Same as app", image: null, tile: null });
  for (const preset of APP_ICON_PRESETS) tiles.push(preset);
  tiles.push({ id: "custom", label: hasCustom ? "Yours" : "Your own", image: null, tile: null, custom: true });
  return tiles;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { APP_ICON_PRESETS, appIconChoices };
}
