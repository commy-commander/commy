const ROOM_MENU_ICONS = { move: "arrow-right", disconnect: "phone-off", kick: "user-x", ban: "ban", room: "volume-2" };
const ROOM_MENU_FACES = "#pt-room .ptile[data-user], #pt-room .pt-aud-chip[data-user]";

function roomMenuRow(menu, icon, words, run, danger = false) {
  const row = el("button", danger ? "danger" : "");
  row.type = "button";
  row.insertAdjacentHTML("beforeend", studioIconMarkup(icon));
  row.appendChild(el("span", "l", words));
  row.onclick = run;
  menu.appendChild(row);
  return row;
}

function showRoomMenu(menu, title) {
  menu.insertBefore(el("div", "ctx-title", title), menu.firstChild);
  placeMenuAt(menu, 0, window.innerHeight);
}

function runRoomMenuAction(kind, server, member) {
  closeFloating();
  if (kind === "disconnect") moveVoiceMember(member, null, server);
  else if (kind === "kick") confirmKick(member, server);
  else if (kind === "ban") confirmBan(member, server);
}

function openRoomMoves(server, member, rooms) {
  closeFloating();
  const menu = $("room-member-menu");
  menu.replaceChildren();
  for (const room of rooms) {
    roomMenuRow(menu, ROOM_MENU_ICONS.room, channelPlainName(room), () => {
      closeFloating();
      moveVoiceMember(member, room, server);
    });
  }
  showRoomMenu(menu, `Move ${member.name} to`);
}

function openRoomMemberMenu(member) {
  const server = roomServer();
  const actions = roomMemberActions(server, { targetId: member.id, myId: state.user && state.user.id, roomId: state.voiceChannelId });
  if (!actions.length) return false;
  closeFloating();
  const menu = $("room-member-menu");
  menu.replaceChildren();
  for (const action of actions) {
    const words = roomMemberActionWords(action.kind, member.name);
    const run = action.kind === "move" ? () => openRoomMoves(server, member, action.rooms) : () => runRoomMenuAction(action.kind, server, member);
    roomMenuRow(menu, ROOM_MENU_ICONS[action.kind], words, run, action.kind !== "move");
  }
  showRoomMenu(menu, member.name);
  return true;
}

function roomFaceMember(face) {
  const server = roomServer();
  const known = server && (server.members || []).find(member => member.id === face.dataset.user);
  return { id: face.dataset.user, name: known ? known.name : face.querySelector("b").textContent };
}

function wireRoomMemberMenu() {
  const room = $("pt-room");
  if (!room) return;
  room.addEventListener("contextmenu", e => {
    const face = e.target.closest && e.target.closest(ROOM_MENU_FACES);
    if (!face) return;
    if (!openRoomMemberMenu(roomFaceMember(face))) return;
    e.preventDefault();
    e.stopPropagation();
  });
  window.addEventListener("popstate", () => $("room-member-menu").classList.add("hidden"));
}

wireRoomMemberMenu();
