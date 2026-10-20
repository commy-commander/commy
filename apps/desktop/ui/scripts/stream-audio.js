const SHARE_AUDIO_KEY = "commy.shareAudio";
const STREAM_VOLUME_MAX_PERCENT = 200;

function shareAudioChoice(saved, supported) {
  if (!supported) return false;
  return saved !== "off";
}

function shareAudioSaveValue(checked) {
  return checked ? "on" : "off";
}

function streamVolumePercent(voice) {
  const gain = voice && Number.isFinite(voice.stream_volume) ? voice.stream_volume : 1;
  return Math.max(0, Math.min(STREAM_VOLUME_MAX_PERCENT, Math.round(gain * 100)));
}

function streamVolumeGain(percent) {
  const value = Number.isFinite(percent) ? percent : 100;
  return Math.max(0, Math.min(STREAM_VOLUME_MAX_PERCENT, value)) / 100;
}

function streamVolumeTitle(percent) {
  return percent === 0 ? "Stream muted" : `Stream volume ${percent}%`;
}

function liveToastText(sourceName, audioShared) {
  return audioShared ? `You are live with audio: ${sourceName}` : `You are live: ${sourceName}`;
}

function liveSourceText(sourceName, audioShared, fps) {
  const streaming = sourceName ? `Streaming ${sourceName}${audioShared ? " with audio" : ""}` : "";
  return [streaming, `${fps} fps`].filter(Boolean).join(" - ");
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { SHARE_AUDIO_KEY, STREAM_VOLUME_MAX_PERCENT, shareAudioChoice, shareAudioSaveValue, streamVolumePercent, streamVolumeGain, streamVolumeTitle, liveToastText, liveSourceText };
}
