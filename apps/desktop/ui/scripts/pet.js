const PET_SUITS = [
  { id: "white", label: "Moon white", color: "#e9e6dc" },
  { id: "orange", label: "Mars orange", color: "#ff8a3d" },
  { id: "blue", label: "Earth blue", color: "#4d9bff" },
  { id: "pink", label: "Nebula pink", color: "#ff6fb5" },
  { id: "green", label: "Aurora green", color: "#3ec98a" },
  { id: "purple", label: "Deep space purple", color: "#9b7bff" },
];
const PET_NAME_MAX = 24;
const PET_HOUR = 60 * 60 * 1000;
const PET_SKEW = 5 * 60 * 1000;
const PET_CARES = [
  { kind: "feed", label: "Feed", field: "fed", done: "munches a space snack" },
  { kind: "pat", label: "Pat", field: "patted", done: "does a happy wiggle" },
  { kind: "play", label: "Play", field: "played", done: "chases a floating ball" },
];

function petSuitColor(suit) {
  return (PET_SUITS.find(s => s.id === suit) || PET_SUITS[0]).color;
}

function petAgo(at, now) {
  if (!at || at > now + PET_SKEW) return Infinity;
  return Math.max(0, now - at);
}

function petMood(pet, now) {
  if (!pet) return null;
  const name = pet.name;
  const last = pet.last || {};
  const fed = petAgo(last.fed, now);
  const fun = Math.min(petAgo(last.patted, now), petAgo(last.played, now));
  const cared = fed !== Infinity || fun !== Infinity;
  if (!cared && now - pet.adopted < PET_HOUR) return { mood: "new", line: `${name} just landed. Say hi!` };
  if (fed > 12 * PET_HOUR) return { mood: "hungry", line: `${name}'s tummy is rumbling.` };
  if (fun > 8 * PET_HOUR) return { mood: "bored", line: `${name} is floating around, looking for something to do.` };
  if (fun < PET_HOUR) return { mood: "happy", line: `${name} is doing zero-g flips.` };
  if (fed < PET_HOUR) return { mood: "full", line: `${name} is full and happy.` };
  return { mood: "content", line: `${name} is watching the stars.` };
}

function petCareWait(pet, kind, now) {
  const care = PET_CARES.find(c => c.kind === kind);
  if (!pet || !care) return 0;
  const mine = (pet.mine || {})[care.field] || 0;
  if (!mine) return 0;
  return Math.max(0, mine + (pet.cooldown_ms || 0) - now);
}

function petCareDone(pet, kind) {
  const care = PET_CARES.find(c => c.kind === kind);
  return pet && care ? `${pet.name} ${care.done}` : "";
}

function petLevelLabel(pet) {
  return pet ? `Level ${pet.level}` : "";
}

function petProgress(pet) {
  if (!pet) return 0;
  const from = 3 * (pet.level - 1) * (pet.level - 1);
  const span = Math.max(1, pet.next_level_xp - from);
  return Math.min(1, Math.max(0, (pet.xp - from) / span));
}

function petCleanName(name) {
  return [...String(name || "").trim()].filter(c => !/[\u0000-\u001f\u007f-\u009f]/.test(c)).slice(0, PET_NAME_MAX).join("");
}

const PET_FACES = {
  new: { eyes: "M26 21.5a1.8 2.4 0 1 0 0.01 0zM38 21.5a1.8 2.4 0 1 0 0.01 0z", mouth: "M29 27.5 Q32 30 35 27.5" },
  happy: { eyes: "M24.5 23 Q26.5 20.5 28.5 23M35.5 23 Q37.5 20.5 39.5 23", mouth: "M28 27 Q32 31.5 36 27" },
  full: { eyes: "M24.5 22.5 Q26.5 21 28.5 22.5M35.5 22.5 Q37.5 21 39.5 22.5", mouth: "M29 27.5 Q32 30 35 27.5" },
  content: { eyes: "M26 21.5a1.8 2.4 0 1 0 0.01 0zM38 21.5a1.8 2.4 0 1 0 0.01 0z", mouth: "M29.5 28 Q32 29.5 34.5 28" },
  hungry: { eyes: "M26 21.5a1.8 2.4 0 1 0 0.01 0zM38 21.5a1.8 2.4 0 1 0 0.01 0z", mouth: "M32 26.5a2 2.2 0 1 0 0.01 0z" },
  bored: { eyes: "M24.5 22.5 H28.5M35.5 22.5 H39.5", mouth: "M29.5 28.5 H34.5" },
};

function petSvg(suit, mood) {
  const face = PET_FACES[mood] || PET_FACES.content;
  const color = petSuitColor(suit);
  return `<svg viewBox="0 0 64 64" class="astro mood-${mood in PET_FACES ? mood : "content"}" aria-hidden="true">`
    + `<g class="astro-float">`
    + `<rect x="15" y="33" width="8" height="16" rx="3" fill="${color}" opacity=".75"/>`
    + `<rect x="11" y="37" width="11" height="6" rx="3" fill="${color}"/>`
    + `<rect x="42" y="37" width="11" height="6" rx="3" fill="${color}"/>`
    + `<rect x="20" y="33" width="24" height="20" rx="8" fill="${color}"/>`
    + `<rect x="23" y="49" width="7" height="10" rx="3.5" fill="${color}"/>`
    + `<rect x="34" y="49" width="7" height="10" rx="3.5" fill="${color}"/>`
    + `<rect x="28" y="38" width="8" height="6" rx="2" fill="#0b0d17" opacity=".35"/>`
    + `<line x1="32" y1="9" x2="32" y2="4" stroke="#f2efe6" stroke-width="2" stroke-linecap="round"/>`
    + `<circle class="astro-light" cx="32" cy="3.5" r="2.5" fill="#ff4d4d"/>`
    + `<circle cx="32" cy="23" r="15" fill="#f2efe6" stroke="${color}" stroke-width="3"/>`
    + `<rect x="21" y="15" width="22" height="16" rx="8" fill="#0b0d17"/>`
    + `<path d="${face.eyes}" fill="#f2efe6" stroke="#f2efe6" stroke-width="1.6" stroke-linecap="round"/>`
    + `<path d="${face.mouth}" fill="none" stroke="#f2efe6" stroke-width="1.6" stroke-linecap="round"/>`
    + `<path d="M24 18 Q27 16 30 16.5" fill="none" stroke="#f2efe6" stroke-width="1.2" stroke-linecap="round" opacity=".45"/>`
    + `</g></svg>`;
}

if (typeof module !== "undefined") {
  module.exports = { PET_SUITS, PET_NAME_MAX, PET_CARES, petSuitColor, petMood, petCareWait, petCareDone, petLevelLabel, petProgress, petCleanName, petSvg };
}
