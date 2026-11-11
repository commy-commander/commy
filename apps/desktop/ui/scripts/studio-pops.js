const POPS_PICKS = [["ap-device", "audio-pop"], ["vpop-device", "video-pop"]];

function popsPickIcon(popId, which) {
  if (popId === "video-pop") return "camera";
  return which === "output" ? "headset" : "mic";
}

function popsUnwrap(select) {
  const box = select.parentElement;
  if (box && box.hasAttribute("data-pops")) box.replaceWith(select);
}

function popsWrap(select, pop) {
  let box = select.parentElement;
  if (!box.hasAttribute("data-pops")) {
    box = document.createElement("div");
    box.className = "pops-pick";
    box.setAttribute("data-pops", "");
    const icon = document.createElement("span");
    icon.className = "pops-icon";
    const chev = document.createElement("span");
    chev.className = "pops-chev";
    chev.appendChild(svgIcon("chevronDown", 18));
    select.replaceWith(box);
    box.append(icon, select, chev);
  }
  const name = popsPickIcon(pop.id, pop.dataset.which);
  if (box.dataset.icon !== name) {
    box.dataset.icon = name;
    box.querySelector(":scope > .pops-icon").replaceChildren(svgIcon(name, 18));
  }
}

function studioPopsDress() {
  const roomy = document.documentElement.dataset.fit !== "slim";
  for (const [selectId, popId] of POPS_PICKS) {
    const select = document.getElementById(selectId);
    const pop = document.getElementById(popId);
    if (!select || !pop) continue;
    if (roomy) popsWrap(select, pop);
    else popsUnwrap(select);
  }
}

function studioPops() {
  studioPopsDress();
  new MutationObserver(studioPopsDress).observe(document.documentElement, { attributes: true, attributeFilter: ["data-fit"] });
  const audio = document.getElementById("audio-pop");
  if (audio) new MutationObserver(studioPopsDress).observe(audio, { attributes: true, attributeFilter: ["data-which"] });
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { POPS_PICKS, popsPickIcon };
}
