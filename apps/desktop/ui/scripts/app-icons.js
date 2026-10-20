const APP_ICON_PRESETS = [
  { id: "classic", label: "Classic", filter: "" },
  { id: "amber", label: "Amber", filter: "hue-rotate(40deg)" },
  { id: "mint", label: "Mint", filter: "hue-rotate(130deg)" },
  { id: "ocean", label: "Ocean", filter: "hue-rotate(200deg)" },
  { id: "violet", label: "Violet", filter: "hue-rotate(270deg)" },
  { id: "mono", label: "Mono", filter: "grayscale(1)" },
];

function appIconChoices(target, hasCustom) {
  const tiles = [];
  if (target === "tray") tiles.push({ id: "app", label: "Same as app", filter: null });
  for (const preset of APP_ICON_PRESETS) tiles.push(preset);
  tiles.push({ id: "custom", label: hasCustom ? "Yours" : "Your own", filter: null, custom: true });
  return tiles;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { APP_ICON_PRESETS, appIconChoices };
}
