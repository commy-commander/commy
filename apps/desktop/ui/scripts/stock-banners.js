const STOCK_BANNERS = [
  { id: "night-peaks", name: "Night peaks" },
  { id: "dusk-dunes", name: "Dusk dunes" },
  { id: "tide", name: "Tide" },
  { id: "aurora", name: "Aurora" },
  { id: "orbit", name: "Orbit" },
  { id: "city-lights", name: "City lights" },
  { id: "deep-sea", name: "Deep sea" },
  { id: "pines", name: "Pines" },
  { id: "meadow", name: "Meadow" },
  { id: "clouds", name: "Clouds" },
  { id: "sunrise", name: "Sunrise" },
  { id: "canyon", name: "Canyon" },
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
