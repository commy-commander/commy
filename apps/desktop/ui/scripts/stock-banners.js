const STOCK_BANNERS = [
  { id: "night-peaks", get name() { return t("ui.night-peaks"); } },
  { id: "dusk-dunes", get name() { return t("ui.dusk-dunes"); } },
  { id: "tide", get name() { return t("ui.tide"); } },
  { id: "aurora", get name() { return t("ui.aurora"); } },
  { id: "orbit", get name() { return t("ui.orbit"); } },
  { id: "city-lights", get name() { return t("ui.city-lights"); } },
  { id: "deep-sea", get name() { return t("ui.deep-sea"); } },
  { id: "pines", get name() { return t("ui.pines"); } },
  { id: "meadow", get name() { return t("ui.meadow"); } },
  { id: "clouds", get name() { return t("ui.clouds"); } },
  { id: "sunrise", get name() { return t("ui.sunrise"); } },
  { id: "canyon", get name() { return t("ui.canyon"); } },
];

function stockBannerSrc(id) {
  return STOCK_BANNERS.some(banner => banner.id === id) ? `images/banners/${id}.svg` : null;
}

function stockBannerTiles(doc, pick) {
  return STOCK_BANNERS.map(banner => {
    const tile = doc.createElement("button");
    tile.type = "button";
    tile.className = "banner-tile";
    tile.title = banner.name;
    tile.setAttribute("aria-label", banner.name);
    const picture = doc.createElement("img");
    picture.src = stockBannerSrc(banner.id);
    picture.alt = "";
    picture.draggable = false;
    tile.appendChild(picture);
    tile.onclick = () => pick(stockBannerSrc(banner.id));
    return tile;
  });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STOCK_BANNERS, stockBannerSrc, stockBannerTiles };
}
