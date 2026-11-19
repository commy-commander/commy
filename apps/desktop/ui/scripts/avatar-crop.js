const AVATAR_CROP_MIN_ZOOM = 1;
const AVATAR_CROP_MAX_ZOOM = 4;
const AVATAR_CROP_OUTPUT = 256;

function avatarCropScale(crop) {
  return Math.max(crop.frameWidth / crop.width, crop.frameHeight / crop.height) * crop.zoom;
}

function clampAvatarCrop(crop) {
  const scale = avatarCropScale(crop);
  const keep = (value, frame, shown) => Math.min(0, Math.max(frame - shown, value));
  return { ...crop, x: keep(crop.x, crop.frameWidth, crop.width * scale), y: keep(crop.y, crop.frameHeight, crop.height * scale) };
}

function avatarCropStart(width, height, frameWidth, frameHeight = frameWidth) {
  const crop = { width, height, frameWidth, frameHeight, zoom: AVATAR_CROP_MIN_ZOOM, x: 0, y: 0 };
  const scale = avatarCropScale(crop);
  return clampAvatarCrop({ ...crop, x: (frameWidth - width * scale) / 2, y: (frameHeight - height * scale) / 2 });
}

function avatarCropMove(crop, dx, dy) {
  return clampAvatarCrop({ ...crop, x: crop.x + dx, y: crop.y + dy });
}

function avatarCropZoom(crop, zoom, at = { x: crop.frameWidth / 2, y: crop.frameHeight / 2 }) {
  const next = Math.min(AVATAR_CROP_MAX_ZOOM, Math.max(AVATAR_CROP_MIN_ZOOM, Number(zoom) || AVATAR_CROP_MIN_ZOOM));
  const ratio = next / crop.zoom;
  return clampAvatarCrop({
    ...crop,
    zoom: next,
    x: at.x - (at.x - crop.x) * ratio,
    y: at.y - (at.y - crop.y) * ratio,
  });
}

function avatarCropSource(crop) {
  const scale = avatarCropScale(crop);
  return { sx: -crop.x / scale, sy: -crop.y / scale, sw: crop.frameWidth / scale, sh: crop.frameHeight / scale };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { AVATAR_CROP_MIN_ZOOM, AVATAR_CROP_MAX_ZOOM, AVATAR_CROP_OUTPUT, avatarCropScale, clampAvatarCrop, avatarCropStart, avatarCropMove, avatarCropZoom, avatarCropSource };
}
