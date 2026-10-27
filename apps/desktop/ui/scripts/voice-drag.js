const VOICE_DRAG = { mouseSlop: 5, touchSlop: 8, touchHoldMs: 350, clickGuardMs: 400, edge: 36, edgeStep: 12 };
const VOICE_MOVE_CHECK_MS = 6000;

function canDragVoiceMember(server, userId, myId) {
  if (!server || !userId) return false;
  if (userId === myId) return true;
  return Boolean(server.can_move && userId !== server.owner);
}

function voiceDragStep(pointerType, dx, dy, heldMs) {
  const far = Math.hypot(dx, dy);
  if (pointerType === "touch") {
    if (far > VOICE_DRAG.touchSlop) return "cancel";
    return heldMs >= VOICE_DRAG.touchHoldMs ? "drag" : "wait";
  }
  return far > VOICE_DRAG.mouseSlop ? "drag" : "wait";
}

function voiceDropRoom(server, fromId, roomId) {
  if (!server || !roomId || roomId === fromId) return null;
  const room = (server.channels || []).find(c => c.id === roomId);
  if (!room || !(room.kind === "voice" || room.kind === "stage") || room.can_view === false) return null;
  return room;
}

function voiceEdgeScroll(y, top, bottom) {
  if (y < top + VOICE_DRAG.edge) return -VOICE_DRAG.edgeStep;
  if (y > bottom - VOICE_DRAG.edge) return VOICE_DRAG.edgeStep;
  return 0;
}

function voiceMoveLanded(server, userId, roomId) {
  const member = server && (server.members || []).find(m => m.id === userId);
  if (!member) return true;
  return (member.voice_channel || null) === (roomId || null);
}

function voiceMoveStuckText(name, roomName) {
  return roomName
    ? `${name} has not switched to ${roomName} yet. Their Commy may need an update.`
    : `${name} is still in voice. Their Commy may need an update.`;
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    VOICE_DRAG, VOICE_MOVE_CHECK_MS, canDragVoiceMember, voiceDragStep, voiceDropRoom, voiceEdgeScroll, voiceMoveLanded, voiceMoveStuckText,
  };
}
