function phoneViewerFit() {
  const box = $("lightbox");
  const img = $("lightbox-img");
  if (studioSlim() && img.naturalWidth && img.naturalHeight) box.style.setProperty("--lb-ratio", img.naturalWidth / img.naturalHeight);
}

const phoneViewer = { dressed: false, step: false };

function phoneViewerIcon(open, slim) {
  const icon = open.querySelector("svg");
  if (icon) icon.replaceWith(slim ? studioIconNode("arrow-up", 15) : svgIcon("openExternal", 16));
}

function phoneViewerDress() {
  const box = $("lightbox");
  const source = $("lightbox-source");
  const tray = box.querySelector(".lightbox-actions");
  const open = $("lightbox-open");
  const slim = studioSlim();
  if (!slim) {
    if (!phoneViewer.dressed) return;
    phoneViewer.dressed = false;
    box.insertBefore(source, tray);
    phoneViewerIcon(open, false);
    return;
  }
  if (!phoneViewer.dressed) {
    phoneViewer.dressed = true;
    tray.prepend(source);
    phoneViewerIcon(open, true);
  }
  const words = el("span", "", source.textContent);
  source.replaceChildren(...(source.classList.contains("linked") ? [svgIcon("link", 14)] : []), words);
  phoneViewerFit();
}

function viewerShown() {
  return studioSlim() && !$("lightbox").classList.contains("hidden");
}

function syncViewerStep() {
  const open = viewerShown();
  const step = layerStep("ptViewer", open, phoneViewer.step, history.state);
  if (step === "push") history.pushState({ ...(history.state || { pane: phonePane() }), ptViewer: true }, "");
  phoneViewer.step = open;
  if (step === "back") history.back();
}

function wirePhoneViewer() {
  const box = $("lightbox");
  if (!box) return;
  $("lightbox-img").addEventListener("load", phoneViewerFit);
  new MutationObserver(() => {
    if (!box.classList.contains("hidden")) phoneViewerDress();
    syncViewerStep();
  }).observe(box, { attributes: true, attributeFilter: ["class"] });
  window.addEventListener("popstate", () => {
    if ((history.state && history.state.ptViewer) || !viewerShown()) return;
    phoneViewer.step = false;
    closeLightbox();
  });
}

wirePhoneViewer();
