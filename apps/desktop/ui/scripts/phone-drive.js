const PHONE_DRIVE_LEAD = "Open this channel in the desktop app to see its files.";

function dressPhoneDrive() {
  if (!studioSlim() || driveWorksHere()) return;
  const empty = $("drive-list").querySelector(":scope > .drive-empty");
  if (!empty || empty.querySelector(":scope > .pt-drive-well")) return;
  const well = el("span", "pt-drive-well");
  well.innerHTML = studioIconMarkup("folder");
  empty.prepend(well, el("b", "", PHONE_DRIVE_LEAD));
}

function wirePhoneDrive() {
  const list = $("drive-list");
  if (!list) return;
  new MutationObserver(dressPhoneDrive).observe(list, { childList: true });
}

wirePhoneDrive();
