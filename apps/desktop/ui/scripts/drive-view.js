const DRIVE_KIND = "drive";
const DRIVE_NAME_MAX = 80;
const DRIVE_DEPTH_MAX = 6;

function isDriveChannel(channel) {
  return Boolean(channel) && channel.kind === DRIVE_KIND;
}

function driveById(items) {
  return new Map((items || []).map(item => [item.id, item]));
}

function driveFolderOf(items, folderId) {
  if (!folderId) return null;
  const found = driveById(items).get(folderId);
  return found && found.folder ? folderId : null;
}

function driveSort(a, b) {
  if (a.folder !== b.folder) return a.folder ? -1 : 1;
  return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
}

function driveChildren(items, parent) {
  const at = parent || null;
  return (items || []).filter(item => (item.parent || null) === at).sort(driveSort);
}

function drivePath(items, folderId) {
  const byId = driveById(items);
  const path = [];
  const seen = new Set();
  let at = folderId ? byId.get(folderId) : null;
  while (at && !seen.has(at.id)) {
    seen.add(at.id);
    path.unshift({ id: at.id, name: at.name });
    at = at.parent ? byId.get(at.parent) : null;
  }
  return path;
}

function driveCrumbs(items, folderId, rootName) {
  return [{ id: null, name: rootName }, ...drivePath(items, folderId)];
}

function driveInside(items, id, ancestor) {
  const byId = driveById(items);
  const seen = new Set();
  let at = byId.get(id);
  while (at && !seen.has(at.id)) {
    if (at.id === ancestor) return true;
    seen.add(at.id);
    at = at.parent ? byId.get(at.parent) : null;
  }
  return false;
}

function driveHeight(items, id) {
  const kids = (items || []).filter(item => item.parent === id);
  return 1 + kids.reduce((most, kid) => Math.max(most, driveHeight(items, kid.id)), 0);
}

function driveMoveTargets(items, item, rootName) {
  const height = item.folder ? driveHeight(items, item.id) : 1;
  const folders = (items || []).filter(f => f.folder && f.id !== item.id && !driveInside(items, f.id, item.id));
  const fits = folders.filter(f => drivePath(items, f.id).length + height <= DRIVE_DEPTH_MAX);
  const labelled = fits.map(f => ({ id: f.id, label: drivePath(items, f.id).map(p => p.name).join(" / ") }));
  labelled.sort((a, b) => a.label.toLowerCase().localeCompare(b.label.toLowerCase()));
  return [{ id: null, label: rootName }, ...labelled];
}

function driveCleanName(raw) {
  const name = String(raw || "").trim();
  if (!name || [...name].length > DRIVE_NAME_MAX || name === "." || name === "..") return null;
  if (/[\\/\u0000-\u001f\u007f]/.test(name)) return null;
  return name;
}

function driveSize(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

function driveUnavailable(item) {
  return !item.folder && !item.kept_here && item.keepers_online === 0;
}

function driveKeptLabel(item, items) {
  if (item.folder) {
    const count = driveChildren(items, item.id).length;
    if (!count) return "Empty";
    return count === 1 ? "1 item" : `${count} items`;
  }
  if (item.kept_here) {
    const others = item.keepers - 1;
    return others > 0 ? `On this device and ${others} other${others === 1 ? "" : "s"}` : "Only on this device";
  }
  if (driveUnavailable(item)) return "Nobody who has it is online";
  const people = item.keepers === 1 ? "1 person" : `${item.keepers} people`;
  return `Kept by ${people}, ${item.keepers_online} online`;
}

function driveRoomLeft(view) {
  return Math.max(0, (view.max_items || 0) - (view.used_items || 0));
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    DRIVE_KIND,
    DRIVE_NAME_MAX,
    DRIVE_DEPTH_MAX,
    isDriveChannel,
    driveFolderOf,
    driveChildren,
    drivePath,
    driveCrumbs,
    driveInside,
    driveMoveTargets,
    driveCleanName,
    driveSize,
    driveUnavailable,
    driveKeptLabel,
    driveRoomLeft,
  };
}
