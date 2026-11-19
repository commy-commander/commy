const NOT_CONNECTED = "(not connected)";
const SAVED_CAMERA_LABEL = "Your camera";

function audioDeviceIdentity(name) {
  return String(name || "").trim().replace(/\(\d+- /g, "(").toLowerCase();
}

function sameAudioDevice(saved, name) {
  if (!saved || !name) return false;
  return saved.trim() === name.trim() || audioDeviceIdentity(saved) === audioDeviceIdentity(name);
}

function connectedAudioDevice(names, saved) {
  if (!saved || !saved.trim()) return null;
  const list = names || [];
  return list.find(n => n === saved.trim()) || list.find(n => sameAudioDevice(saved, n)) || null;
}

function audioDevicePicks(names, saved) {
  const list = names || [];
  const options = [{ value: "", label: t("ui.default") }, ...list.map(n => ({ value: n, label: n }))];
  const wanted = saved && saved.trim() ? saved.trim() : "";
  if (!wanted) return { options, value: "" };
  const connected = connectedAudioDevice(list, wanted);
  if (connected) return { options, value: connected };
  options.push({ value: wanted, label: `${wanted} ${NOT_CONNECTED}` });
  return { options, value: wanted };
}

function readCameraChoice(raw) {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === "object" && typeof parsed.id === "string" && parsed.id) {
      return { id: parsed.id, label: typeof parsed.label === "string" ? parsed.label : "" };
    }
  } catch {}
  return typeof raw === "string" && !raw.startsWith("{") ? { id: raw, label: "" } : null;
}

function cameraChoiceText(choice) {
  return choice && choice.id ? JSON.stringify({ id: choice.id, label: choice.label || "" }) : "";
}

function findCamera(cameras, choice) {
  if (!choice) return null;
  const list = (cameras || []).filter(c => c.deviceId);
  return list.find(c => c.deviceId === choice.id) || (choice.label ? list.find(c => c.label === choice.label) : null) || null;
}

function cameraLabel(camera, index) {
  return camera.label || t("ui.camera", { value: index + 1 });
}

function cameraPicks(cameras, choice) {
  const list = cameras || [];
  const options = [{ value: "", label: t("ui.default") }, ...list.map((c, i) => ({ value: c.deviceId, label: cameraLabel(c, i) }))];
  if (!choice) return { options, value: "" };
  const found = findCamera(list, choice);
  if (found) return { options, value: found.deviceId };
  options.push({ value: choice.id, label: `${choice.label || SAVED_CAMERA_LABEL} ${NOT_CONNECTED}` });
  return { options, value: choice.id };
}

function cameraChoiceFor(cameras, id, previous) {
  if (!id) return null;
  if (previous && previous.id === id) {
    const found = findCamera(cameras, previous);
    return { id, label: (found && found.label) || previous.label };
  }
  const picked = (cameras || []).find(c => c.deviceId === id);
  return { id, label: picked ? picked.label : "" };
}

function cameraMove(cameras, choice, usingId) {
  const list = (cameras || []).filter(c => c.deviceId);
  const wanted = findCamera(list, choice);
  if (wanted) return wanted.deviceId === usingId ? null : { deviceId: wanted.deviceId };
  if (usingId && list.some(c => c.deviceId === usingId)) return null;
  return list.length ? { deviceId: "" } : null;
}

const CAMERA_FACING_WORDS = {
  user: /\b(front|user|facetime|selfie)\b/i,
  environment: /\b(back|rear|environment|world)\b/i,
};

const CAMERA_FACING_PICKS = [
  { value: "user", get label() { return t("ui.front-camera"); } },
  { value: "environment", get label() { return t("ui.back-camera"); } },
];

function readCameraFacing(raw) {
  return raw === "environment" ? "environment" : "user";
}

function facingPicks(facing) {
  return { options: CAMERA_FACING_PICKS.map(pick => ({ ...pick })), value: readCameraFacing(facing) };
}

function otherFacing(facing) {
  return facing === "environment" ? "user" : "environment";
}

function cameraFacing(settings, fallback) {
  const facing = settings && settings.facingMode;
  return facing === "user" || facing === "environment" ? facing : fallback;
}

function cameraForFacing(cameras, facing) {
  const words = CAMERA_FACING_WORDS[facing];
  if (!words) return null;
  const found = (cameras || []).find(c => c.deviceId && words.test(c.label || ""));
  return found ? found.deviceId : null;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    NOT_CONNECTED,
    audioDeviceIdentity, sameAudioDevice, connectedAudioDevice, audioDevicePicks,
    readCameraChoice, cameraChoiceText, findCamera, cameraPicks, cameraChoiceFor, cameraMove,
    CAMERA_FACING_PICKS, readCameraFacing, facingPicks, otherFacing, cameraFacing, cameraForFacing,
  };
}
