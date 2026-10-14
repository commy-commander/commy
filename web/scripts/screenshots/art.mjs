// Generated demo art for screenshots. Everything is drawn here, no stock images.

const PALETTES = [
  ["#ff6b6b", "#ffb36b"], ["#6b8bff", "#a46bff"], ["#2fd3a7", "#2f8bd3"], ["#ffcf4d", "#ff6b9e"],
  ["#9d6bff", "#ff6bd6"], ["#4dd0ff", "#4dff9d"], ["#ff8a4d", "#d34dff"], ["#5ee0c0", "#e0d35e"],
];

const GLYPHS = [
  // ring, triangle, diamond, wave, star, bolt, hex, moon
  '<circle cx="64" cy="64" r="26" fill="none" stroke="#fff" stroke-width="10"/>',
  '<path d="M64 34 L94 88 H34 Z" fill="none" stroke="#fff" stroke-width="9" stroke-linejoin="round"/>',
  '<path d="M64 32 L96 64 L64 96 L32 64 Z" fill="#fff" opacity=".92"/>',
  '<path d="M30 70 Q47 46 64 70 T98 70" fill="none" stroke="#fff" stroke-width="10" stroke-linecap="round"/>',
  '<path d="M64 30 L73 54 L99 55 L79 71 L86 96 L64 82 L42 96 L49 71 L29 55 L55 54 Z" fill="#fff" opacity=".92"/>',
  '<path d="M70 28 L40 70 H62 L56 100 L88 56 H66 Z" fill="#fff" opacity=".92"/>',
  '<path d="M64 32 L92 48 V80 L64 96 L36 80 V48 Z" fill="none" stroke="#fff" stroke-width="9" stroke-linejoin="round"/>',
  '<path d="M78 36 A32 32 0 1 0 92 82 A26 26 0 1 1 78 36 Z" fill="#fff" opacity=".92"/>',
];

export function avatarSvg(i) {
  const [a, b] = PALETTES[i % PALETTES.length];
  return `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient></defs>
<rect width="128" height="128" fill="url(#g)"/>${GLYPHS[i % GLYPHS.length]}</svg>`;
}

export function serverIconSvg() {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#7c5cff"/><stop offset="1" stop-color="#ff5c8a"/></linearGradient></defs>
<rect width="256" height="256" fill="url(#g)"/>
<g fill="#fff"><rect x="68" y="68" width="40" height="40" rx="6"/><rect x="148" y="68" width="40" height="40" rx="6" opacity=".7"/>
<rect x="68" y="148" width="40" height="40" rx="6" opacity=".7"/><rect x="148" y="148" width="40" height="40" rx="6"/></g></svg>`;
}

// A made-up racing game frame for the screen share demo.
export function gameSvg() {
  const lines = [];
  for (let i = 0; i <= 16; i++) {
    const x = -640 + i * 160;
    lines.push(`<line x1="640" y1="420" x2="${x}" y2="720" stroke="#ff4fd8" stroke-opacity=".55" stroke-width="2"/>`);
  }
  for (let i = 0; i < 9; i++) {
    const y = 420 + Math.pow(i / 8, 2) * 300;
    lines.push(`<line x1="0" y1="${y.toFixed(1)}" x2="1280" y2="${y.toFixed(1)}" stroke="#ff4fd8" stroke-opacity=".45" stroke-width="2"/>`);
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720" viewBox="0 0 1280 720">
<defs>
<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#120b2e"/><stop offset=".6" stop-color="#4a1260"/><stop offset="1" stop-color="#ff5e7e"/></linearGradient>
<linearGradient id="sun" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffe45e"/><stop offset="1" stop-color="#ff4f9a"/></linearGradient>
<linearGradient id="floor" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#1b0833"/><stop offset="1" stop-color="#07020f"/></linearGradient>
<linearGradient id="road" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a1550"/><stop offset="1" stop-color="#120626"/></linearGradient>
<clipPath id="sunclip"><rect x="0" y="0" width="1280" height="420"/></clipPath>
</defs>
<rect width="1280" height="420" fill="url(#sky)"/>
<g fill="#fff" opacity=".8"><circle cx="120" cy="60" r="1.6"/><circle cx="300" cy="120" r="1.2"/><circle cx="520" cy="40" r="1.4"/><circle cx="880" cy="90" r="1.6"/><circle cx="1040" cy="40" r="1.2"/><circle cx="1180" cy="140" r="1.4"/><circle cx="210" cy="190" r="1"/><circle cx="980" cy="200" r="1"/></g>
<g clip-path="url(#sunclip)"><circle cx="640" cy="380" r="170" fill="url(#sun)"/>
<g fill="#4a1260"><rect x="440" y="330" width="400" height="8"/><rect x="440" y="356" width="400" height="11"/><rect x="440" y="384" width="400" height="14"/><rect x="440" y="408" width="400" height="12"/></g></g>
<path d="M0 420 L120 330 L210 380 L330 290 L450 400 L520 360 L560 420 Z" fill="#26093f"/>
<path d="M720 420 L790 350 L880 395 L990 300 L1110 380 L1200 320 L1280 360 L1280 420 Z" fill="#26093f"/>
<rect y="420" width="1280" height="300" fill="url(#floor)"/>
${lines.join("\n")}
<path d="M600 420 L680 420 L1000 720 L280 720 Z" fill="url(#road)" opacity=".85"/>
<g fill="#ffe45e"><rect x="636" y="440" width="8" height="18"/><rect x="633" y="490" width="14" height="30"/><rect x="628" y="570" width="24" height="50"/></g>
<g transform="translate(640 610)">
<path d="M-120 60 L-90 -10 L90 -10 L120 60 Z" fill="#14e1ff"/>
<path d="M-70 -10 L-40 -50 L40 -50 L70 -10 Z" fill="#0a7ea0"/>
<path d="M-52 -14 L-34 -42 L34 -42 L52 -14 Z" fill="#a8f3ff" opacity=".7"/>
<rect x="-118" y="40" width="40" height="12" rx="3" fill="#ff4f9a"/><rect x="78" y="40" width="40" height="12" rx="3" fill="#ff4f9a"/>
<rect x="-100" y="60" width="200" height="10" fill="#ff4f9a" opacity=".5"/>
</g>
<g font-family="Arial, sans-serif" font-weight="700" fill="#fff">
<text x="40" y="58" font-size="30" letter-spacing="2">LAP 2/3</text>
<text x="40" y="92" font-size="20" fill="#14e1ff" letter-spacing="1">01:12.48</text>
<text x="1240" y="58" font-size="30" text-anchor="end" letter-spacing="2">POS 1/8</text>
<text x="1240" y="680" font-size="64" text-anchor="end">218</text>
<text x="1240" y="702" font-size="18" text-anchor="end" fill="#14e1ff" letter-spacing="3">KM/H</text>
</g>
<g transform="translate(40 560)"><rect width="200" height="120" rx="10" fill="#000" opacity=".45"/>
<path d="M30 90 C30 40 80 30 100 50 S170 80 170 30" fill="none" stroke="#fff" stroke-opacity=".7" stroke-width="5" stroke-linecap="round"/>
<circle cx="140" cy="62" r="7" fill="#14e1ff"/><circle cx="76" cy="38" r="5" fill="#ff4f9a"/></g>
</svg>`;
}

// A made-up "look what we built" screenshot shared in chat.
export function islandSvg() {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="450" viewBox="0 0 800 450">
<defs>
<linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#7fd4ff"/><stop offset="1" stop-color="#d9f3ff"/></linearGradient>
<linearGradient id="w" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2fa3d9"/><stop offset="1" stop-color="#155d9c"/></linearGradient>
</defs>
<rect width="800" height="450" fill="url(#s)"/>
<g fill="#fff" opacity=".9"><ellipse cx="160" cy="80" rx="60" ry="18"/><ellipse cx="200" cy="70" rx="40" ry="20"/><ellipse cx="620" cy="110" rx="70" ry="16"/></g>
<rect y="290" width="800" height="160" fill="url(#w)"/>
<path d="M150 300 L260 230 L350 250 L470 190 L560 240 L660 300 Z" fill="#3fa34d"/>
<path d="M150 300 L660 300 L620 330 L190 330 Z" fill="#e8d18f"/>
<path d="M260 230 L350 250 L300 300 L200 300 Z" fill="#358c41"/>
<path d="M470 190 L560 240 L520 300 L430 300 Z" fill="#2f7a3a"/>
<g transform="translate(400 170)">
<rect x="-40" y="0" width="80" height="80" fill="#c9a37a"/><path d="M-50 0 L0 -40 L50 0 Z" fill="#b5523b"/>
<rect x="-12" y="40" width="24" height="40" fill="#6b4a2f"/><rect x="-32" y="14" width="16" height="16" fill="#ffe9a8"/><rect x="16" y="14" width="16" height="16" fill="#ffe9a8"/>
</g>
<g fill="#2f7a3a"><circle cx="240" cy="270" r="18"/><circle cx="560" cy="262" r="20"/><circle cx="600" cy="280" r="14"/></g>
<g fill="#6b4a2f"><rect x="237" y="280" width="6" height="20"/><rect x="557" y="276" width="6" height="22"/></g>
<g stroke="#fff" stroke-opacity=".5" stroke-width="3" fill="none"><path d="M60 360 q20 -8 40 0"/><path d="M700 380 q20 -8 40 0"/><path d="M420 410 q20 -8 40 0"/></g>
</svg>`;
}

export function svgDataUrl(svg) {
  return "data:image/svg+xml;base64," + Buffer.from(svg).toString("base64");
}
