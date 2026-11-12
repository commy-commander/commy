const APP_ICON_MARK = "images/commy-star.svg";

const APP_ICON_PRESETS = [
  { id: "classic", label: "Night", image: APP_ICON_MARK, tile: "#1a2574" },
  { id: "amber", label: "Amber", image: APP_ICON_MARK, tile: "#f2b24c" },
  { id: "mint", label: "Mint", image: APP_ICON_MARK, tile: "#8fdcbd" },
  { id: "ocean", label: "Sky", image: APP_ICON_MARK, tile: "#c3d6ff" },
  { id: "violet", label: "Lilac", image: APP_ICON_MARK, tile: "#dccdff" },
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
