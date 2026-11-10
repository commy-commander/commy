const STUDIO_FACES = ["fox", "owl", "finch", "visor", "moon", "frog", "cat", "bear", "rabbit", "planet"];
const STUDIO_BOT_FACE = "bot";
const STUDIO_FACE_COLOURS = ["ferris", "nova", "pip", "ash", "lumen", "dot"];
const STUDIO_FACE_SPRITE = "studio-faces";

const STUDIO_FACE_SHARED = '<g id="sf-cap"><rect width="40" height="40" rx="11" style="fill:color-mix(in srgb, var(--c) 24%, var(--a-ground))"/><path d="M0 11A11 11 0 0 1 11 0H40L0 30Z" style="fill:var(--white)" opacity=".045"/></g><g id="sf-eye"><rect x="-1.5" y="-2.7" width="3" height="5.4" rx="1.5" style="fill:var(--a-ground)"/></g><clipPath id="sf-clip"><rect width="40" height="40" rx="11"/></clipPath>';

const STUDIO_FACE_ART = {
  fox: '<use href="#sf-cap"/><path d="M8.5 9.5 16 16h8l7.5-6.5V23L20 33.5 8.5 23Z" style="fill:var(--c)"/><path d="M20 16h4l7.5-6.5V23L20 33.5Z" style="fill:var(--a-ground)" opacity=".2"/><path d="M17.6 30.6h4.8L20 33.5Z" style="fill:var(--a-ground)"/><use href="#sf-eye" x="14.8" y="21.4"/><use href="#sf-eye" x="25.2" y="21.4"/>',
  owl: '<use href="#sf-cap"/><g clip-path="url(#sf-clip)"><path d="M7.5 41V12l5.5 3.4h14L32.5 12v29Z" style="fill:var(--c)"/><path d="M20 15.4h7l5.5-3.4v29H20Z" style="fill:var(--a-ground)" opacity=".18"/></g><circle cx="14.4" cy="21.4" r="5.2" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))"/><circle cx="25.6" cy="21.4" r="5.2" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))"/><use href="#sf-eye" x="15" y="21.4"/><use href="#sf-eye" x="25" y="21.4"/><path d="M18.3 26.6h3.4L20 30Z" style="fill:var(--a-ground)"/>',
  finch: '<use href="#sf-cap"/><g clip-path="url(#sf-clip)"><path d="M4 41c0-10 5.5-15 13-15s11 4 11 15Z" style="fill:var(--c)" opacity=".62"/></g><circle cx="18.5" cy="17.6" r="9.4" style="fill:var(--c)"/><path d="M26.6 14.4 35.5 18l-8.9 3.6Z" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))"/><use href="#sf-eye" x="21" y="16.2"/>',
  visor: '<use href="#sf-cap"/><g clip-path="url(#sf-clip)"><path d="M8.5 41V20a11.5 11.5 0 0 1 23 0v21Z" style="fill:var(--c)"/><path d="M20 8.5a11.5 11.5 0 0 1 11.5 11.5v21H20Z" style="fill:var(--a-ground)" opacity=".16"/></g><rect x="10.6" y="16" width="18.8" height="8" rx="4" style="fill:var(--a-ground)"/><path d="M14 18h5.6l-2.4 4h-5.6Z" style="fill:var(--c)" opacity=".5"/>',
  moon: '<use href="#sf-cap"/><path d="M24.5 8.2a12.2 12.2 0 1 0 7.3 20.6 10 10 0 0 1-7.3-20.6Z" style="fill:var(--c)"/><use href="#sf-eye" x="17.6" y="19.6"/><path d="M29.5 11.5l1 2.5 2.5 1-2.5 1-1 2.5-1-2.5-2.5-1 2.5-1Z" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))"/>',
  frog: '<use href="#sf-cap"/><g clip-path="url(#sf-clip)"><path d="M6 41V27a14 10.5 0 0 1 28 0v14Z" style="fill:var(--c)"/><path d="M20 16.5a14 10.5 0 0 1 14 10.5v14H20Z" style="fill:var(--a-ground)" opacity=".16"/></g><circle cx="13" cy="16.5" r="5.6" style="fill:var(--c)"/><circle cx="27" cy="16.5" r="5.6" style="fill:var(--c)"/><use href="#sf-eye" x="13.4" y="16.5"/><use href="#sf-eye" x="26.6" y="16.5"/><path d="M15 29.5q5 2.6 10 0" fill="none" style="stroke:var(--a-ground)" stroke-width="1.5" stroke-linecap="round" opacity=".7"/>',
  cat: '<use href="#sf-cap"/><path d="M8.6 20V7.5l8.4 6.5ZM31.4 20V7.5L23 14Z" style="fill:var(--c)"/><path d="M10.8 15v-3.6l3 2.3ZM29.2 15v-3.6l-3 2.3Z" style="fill:var(--a-ground)" opacity=".3"/><ellipse cx="20" cy="23.5" rx="12.4" ry="10.5" style="fill:var(--c)"/><path d="M20 13a12.4 10.5 0 0 1 0 21Z" style="fill:var(--a-ground)" opacity=".16"/><path d="M17.2 14.5v2.8M20 14v3.8M22.8 14.5v2.8" fill="none" style="stroke:var(--a-ground)" stroke-width="1.4" stroke-linecap="round" opacity=".45"/><path d="M10.5 26.2 5 25.2M10.5 28.6 5 29.8M29.5 26.2 35 25.2M29.5 28.6 35 29.8" fill="none" style="stroke:color-mix(in srgb, var(--c) 30%, var(--white))" stroke-width="1.2" stroke-linecap="round"/><use href="#sf-eye" x="14.8" y="22.8"/><use href="#sf-eye" x="25.2" y="22.8"/><path d="M18.3 26.8h3.4L20 29Z" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))"/>',
  bear: '<use href="#sf-cap"/><circle cx="11.5" cy="12.5" r="5" style="fill:var(--c)"/><circle cx="28.5" cy="12.5" r="5" style="fill:var(--c)"/><circle cx="11.5" cy="12.5" r="2.2" style="fill:var(--a-ground)" opacity=".3"/><circle cx="28.5" cy="12.5" r="2.2" style="fill:var(--a-ground)" opacity=".3"/><circle cx="20" cy="22.5" r="12" style="fill:var(--c)"/><path d="M20 10.5a12 12 0 0 1 0 24Z" style="fill:var(--a-ground)" opacity=".16"/><ellipse cx="20" cy="27.2" rx="5.6" ry="4.4" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))"/><ellipse cx="20" cy="25.6" rx="2" ry="1.4" style="fill:var(--a-ground)"/><use href="#sf-eye" x="14.6" y="20.2"/><use href="#sf-eye" x="25.4" y="20.2"/>',
  rabbit: '<use href="#sf-cap"/><rect x="11.2" y="4.5" width="6.6" height="19" rx="3.3" style="fill:var(--c)"/><rect x="22.2" y="4.5" width="6.6" height="19" rx="3.3" style="fill:var(--c)"/><rect x="13.5" y="7.5" width="2" height="10" rx="1" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))" opacity=".7"/><rect x="24.5" y="7.5" width="2" height="10" rx="1" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))" opacity=".7"/><g clip-path="url(#sf-clip)"><path d="M8 41V28.5a12 10.5 0 0 1 24 0V41Z" style="fill:var(--c)"/><path d="M20 18a12 10.5 0 0 1 12 10.5V41H20Z" style="fill:var(--a-ground)" opacity=".16"/></g><use href="#sf-eye" x="14.8" y="27"/><use href="#sf-eye" x="25.2" y="27"/><path d="M18.4 31h3.2L20 33Z" style="fill:var(--a-ground)"/>',
  planet: '<use href="#sf-cap"/><g transform="rotate(-18 20 21)"><ellipse cx="20" cy="21" rx="16.5" ry="5" fill="none" style="stroke:color-mix(in srgb, var(--c) 30%, var(--white))" stroke-width="2.2" opacity=".55"/></g><circle cx="20" cy="21" r="10.2" style="fill:var(--c)"/><path d="M20 10.8a10.2 10.2 0 0 1 0 20.4Z" style="fill:var(--a-ground)" opacity=".16"/><g transform="rotate(-18 20 21)"><path d="M3.5 21a16.5 5 0 0 0 33 0" fill="none" style="stroke:color-mix(in srgb, var(--c) 30%, var(--white))" stroke-width="2.2" stroke-linecap="round"/></g><use href="#sf-eye" x="16.4" y="17.8"/><use href="#sf-eye" x="23.6" y="17.8"/>',
  bot: '<use href="#sf-cap"/><path d="M20 12.5V8" fill="none" style="stroke:var(--c)" stroke-width="1.6" stroke-linecap="round"/><circle cx="20" cy="7.4" r="1.9" style="fill:color-mix(in srgb, var(--c) 30%, var(--white))"/><rect x="8.5" y="12.5" width="23" height="19" rx="5" style="fill:var(--c)"/><rect x="11.5" y="16.5" width="17" height="8" rx="2.6" style="fill:var(--a-ground)"/><rect x="14.4" y="18.7" width="2.6" height="3.6" rx="1.3" style="fill:var(--a-eye, var(--green))"/><rect x="23" y="18.7" width="2.6" height="3.6" rx="1.3" style="fill:var(--a-eye, var(--green))"/><path d="M16 28h8" fill="none" style="stroke:var(--a-ground)" stroke-width="1.4" stroke-linecap="round" opacity=".6"/>',
};

function studioFaceHash(id) {
  let hash = 2166136261;
  for (const ch of String(id)) {
    hash ^= ch.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function studioFaceFor(id, bot = false) {
  if (bot) return STUDIO_BOT_FACE;
  return STUDIO_FACES[(studioFaceHash(id) & 65535) % STUDIO_FACES.length];
}

function studioFaceColour(id) {
  return STUDIO_FACE_COLOURS[(studioFaceHash(id) >>> 16) % STUDIO_FACE_COLOURS.length];
}

function studioFaceSprite() {
  const symbols = Object.entries(STUDIO_FACE_ART).map(([name, art]) => `<symbol id="sf-${name}" viewBox="0 0 40 40">${art}</symbol>`).join("");
  return `<svg id="${STUDIO_FACE_SPRITE}" width="0" height="0" aria-hidden="true" focusable="false" style="position:absolute"><defs>${STUDIO_FACE_SHARED}${symbols}</defs></svg>`;
}

function mountStudioFaces(doc) {
  if (!doc.body || doc.getElementById(STUDIO_FACE_SPRITE)) return;
  doc.body.insertAdjacentHTML("afterbegin", studioFaceSprite());
}

function studioFaceNode(doc, id, bot = false) {
  mountStudioFaces(doc);
  const name = studioFaceFor(id, bot);
  const svg = doc.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "avatar-face");
  svg.setAttribute("viewBox", "0 0 40 40");
  svg.setAttribute("aria-hidden", "true");
  svg.dataset.face = name;
  svg.style.setProperty("--c", `var(--role-${studioFaceColour(id)})`);
  const use = doc.createElementNS("http://www.w3.org/2000/svg", "use");
  use.setAttribute("href", `#sf-${name}`);
  svg.appendChild(use);
  return svg;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_FACES, STUDIO_BOT_FACE, STUDIO_FACE_COLOURS, STUDIO_FACE_SPRITE, STUDIO_FACE_SHARED, STUDIO_FACE_ART, studioFaceHash, studioFaceFor, studioFaceColour, studioFaceSprite, mountStudioFaces, studioFaceNode };
}
