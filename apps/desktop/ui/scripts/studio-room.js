const STUDIO_ROOM_SVG = "<svg viewBox='0 0 1440 900' preserveAspectRatio='xMidYMid slice' aria-hidden='true'><defs><linearGradient id='rm-wall' x1='0' y1='0' x2='0' y2='1'><stop offset='0' style='stop-color:var(--bg-darkest)'/><stop offset='.55' style='stop-color:var(--bg-dark)'/><stop offset='1' style='stop-color:var(--section)'/></linearGradient><radialGradient id='rm-pool'><stop offset='0' style='stop-color:var(--wall-lit)'/><stop offset='.5' style='stop-color:var(--wall-lit)' stop-opacity='.6'/><stop offset='1' style='stop-color:var(--wall-lit)' stop-opacity='0'/></radialGradient><linearGradient id='rm-shaft' gradientUnits='userSpaceOnUse' x1='200' y1='0' x2='1100' y2='700'><stop offset='0' style='stop-color:var(--white)' stop-opacity='.2'/><stop offset='.6' style='stop-color:var(--white)' stop-opacity='.09'/><stop offset='1' style='stop-color:var(--white)' stop-opacity='0'/></linearGradient><linearGradient id='rm-floor' x1='0' y1='0' x2='0' y2='1'><stop offset='0' style='stop-color:color-mix(in srgb, var(--wall-lit) 45%, var(--bg-dark))'/><stop offset='.6' style='stop-color:var(--bg-dark)'/><stop offset='1' style='stop-color:var(--bg-darkest)'/></linearGradient><linearGradient id='rm-hz' x1='0' y1='0' x2='1' y2='0'><stop offset='0' style='stop-color:var(--tint-1)' stop-opacity='.25'/><stop offset='.3' style='stop-color:var(--tint-1)'/><stop offset='.72' style='stop-color:var(--accent-lit)'/><stop offset='1' style='stop-color:var(--tint-1)' stop-opacity='.35'/></linearGradient><linearGradient id='rm-door' x1='0' y1='0' x2='0' y2='1'><stop offset='0' style='stop-color:var(--tint-2)' stop-opacity='.35'/><stop offset='1' style='stop-color:color-mix(in srgb, var(--tint-2) 70%, var(--white))'/></linearGradient><linearGradient id='rm-spill' x1='0' y1='0' x2='0' y2='1'><stop offset='0' style='stop-color:var(--tint-2)' stop-opacity='.5'/><stop offset='1' style='stop-color:var(--tint-2)' stop-opacity='0'/></linearGradient><radialGradient id='rm-vig' cx='.5' cy='.45' r='.75'><stop offset='.5' style='stop-color:var(--bg-darkest)' stop-opacity='0'/><stop offset='1' style='stop-color:var(--bg-darkest)' stop-opacity='.7'/></radialGradient><filter id='rm-b3' x='-20%' y='-20%' width='140%' height='140%'><feGaussianBlur stdDeviation='2.5'/></filter><filter id='rm-b14' x='-20%' y='-60%' width='140%' height='220%'><feGaussianBlur stdDeviation='14'/></filter><filter id='rm-b40' x='-30%' y='-200%' width='160%' height='500%'><feGaussianBlur stdDeviation='40'/></filter><clipPath id='rm-wallclip'><rect x='-100' y='-100' width='1640' height='780'/></clipPath><clipPath id='rm-floorclip'><rect x='-100' y='680' width='1640' height='400'/></clipPath></defs><rect x='-50' y='-50' width='1540' height='1000' style='fill:var(--bg-darkest)'/><g class='rm-all'><rect x='-50' y='-250' width='1540' height='1400' fill='url(#rm-wall)'/><ellipse class='rm-pool' cx='600' cy='320' rx='1100' ry='640' fill='url(#rm-pool)'/><g class='rm-shaft' clip-path='url(#rm-wallclip)'><g filter='url(#rm-b14)'><path d='M-80 60 430-140 1210 680H500Z' fill='url(#rm-shaft)'/><path d='M500-170 720-256 1560 680H1290Z' fill='url(#rm-shaft)' opacity='.7'/></g></g><g class='rm-door'><rect x='1204' y='392' width='78' height='288' fill='url(#rm-door)' filter='url(#rm-b3)' opacity='.62'/><ellipse cx='1244' cy='560' rx='150' ry='220' style='fill:var(--tint-2)' opacity='.12' filter='url(#rm-b40)'/></g><g class='rm-floor'><rect x='-50' y='680' width='1540' height='300' fill='url(#rm-floor)'/><g clip-path='url(#rm-floorclip)'><g filter='url(#rm-b14)'><path d='M500 680H1210L1500 1000H640Z' style='fill:var(--white)' opacity='.05'/></g><path class='rm-spill' d='M1204 680h78l90 240h-240Z' fill='url(#rm-spill)' filter='url(#rm-b14)' opacity='.5'/></g><rect class='rm-hz-glow' x='-50' y='610' width='1540' height='130' fill='url(#rm-hz)' opacity='.42' filter='url(#rm-b40)'/><rect class='rm-hz-soft' x='-50' y='672' width='1540' height='16' fill='url(#rm-hz)' opacity='.5' filter='url(#rm-b14)'/><rect class='rm-hz-line' x='-50' y='679' width='1540' height='2' fill='url(#rm-hz)'/></g></g><rect class='rm-vig' x='-50' y='-50' width='1540' height='1000' fill='url(#rm-vig)'/></svg>";

function studioRoomMarkup() {
  return STUDIO_ROOM_SVG;
}

function mountStudioRoom(doc) {
  const root = doc.documentElement;
  if (root.dataset.look !== "studio") return null;
  const present = doc.querySelector(".studio-room");
  if (present) return present;
  const room = doc.createElement("div");
  room.className = "studio-room";
  room.setAttribute("aria-hidden", "true");
  room.innerHTML = studioRoomMarkup();
  doc.body.prepend(room);
  return room;
}

function mountStudioStar(doc, star) {
  if (doc.documentElement.dataset.look !== "studio" || !star) return null;
  const present = doc.querySelector(".studio-star");
  if (present) return present;
  const actions = doc.querySelector(".chat-header .header-actions");
  if (!actions) return null;
  const key = doc.createElement("button");
  key.type = "button";
  key.className = "studio-star";
  key.tabIndex = -1;
  key.setAttribute("aria-hidden", "true");
  const art = doc.createElement("span");
  art.className = "star";
  art.setAttribute("data-state", "idle");
  art.setAttribute("data-lift", "");
  key.appendChild(art);
  actions.before(key);
  star.mount(art);
  return key;
}

function studioStarState(doc, star, state) {
  const art = star ? doc.querySelector(".studio-star .star") : null;
  if (!art) return false;
  star.set(art, state);
  return true;
}

if (typeof document !== "undefined") {
  mountStudioRoom(document);
  mountStudioStar(document, typeof Star === "undefined" ? null : Star);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { STUDIO_ROOM_SVG, studioRoomMarkup, mountStudioRoom, mountStudioStar, studioStarState };
}
