# Voice seats

A seat is the answer to "who is in which voice channel right now". Every app
(desktop, phone, browser) keeps its own copy in `ServerState::voice_states`
and must reach the same answer from the same news. These are the rules.
Change them only together with the tests named at the end.

## Who speaks for a seat

1. An account can be signed in on several devices, but only one of them is in
   voice at a time: the **voice device**.
2. Each device says where it is with a signed `DeviceAnnounce { node, voice,
   since }`. `since` is the moment that device joined, in milliseconds.
3. The device registry (`commy_core::devices`) holds at most one voice device
   per account. A device that announces a voice room takes the seat when
   nobody holds it, when it already holds it, or when its `since` is later
   than the holder's (a tie keeps the holder). Every other device of that account is then stood up.
   An older join heard late does not take the seat back. The devices
   themselves follow the same rule (`devices::takes_over`), so the device that
   loses stops its call on its own.
4. While a voice device is known, the registry decides the room. A
   `VoiceState` that names a different room, or `None`, is ignored, because
   it came from a device that is not in voice or from before a hand-over. A
   `VoiceState` for the same room still updates mute, deafen and the time
   joined.
5. When the voice device changes, the per-account freshness mark for
   `voice:{user}` is cleared. This lets the new device's `VoiceState` count
   even when its clock is behind the old device's.
6. Members whose apps never announce devices keep the older behaviour:
   `VoiceState` alone seats them, moves them and clears them.

## Joining, moving and leaving

- Join or move: stamp the join with `ServerState::voice_join_stamp`, which is
  the local time but always later than the seat it takes over, so a device
  with a slow clock still wins the hand-over. Then send `DeviceAnnounce` with
  the room first, and `VoiceState` after it.
- Leave: send `DeviceAnnounce` with `voice: None` and `VoiceState` with
  `channel: None`. The announce is what ends the seat for apps that know the
  device. The `VoiceState` is for older apps.
- Hand-over to another device of the same account: the old device stops
  locally and announces `voice: None`. It never sends `VoiceState None`.
- Moves by a moderator (`VoiceMove`) never change seats. Only the moved
  member's voice device follows and re-announces. AFK works the same way: only
  the voice device moves itself (`commy_view::afk::idle_voice_step`).

## How a seat ends without a goodbye

A seat must never outlive the device holding it. `commy_view::liveness` ends
a seat when any of these happens:

- the voice device's own `PresenceUpdate` says `Offline`. This covers quitting
  the app and the sync service's will when a connection drops. A goodbye
  signed before something newer from that device is stale and ignored.
  Appearing offline (`Invisible`) never ends a seat.
- nobody has heard from the member for `EXPIRE` (2 minutes), or 11 minutes
  when they were last heard only through the sync service.
- the member is still heard from another device, but the voice device itself
  has been quiet for the same window.
- nobody has re-confirmed the seat for the same window. A seat is confirmed
  only by news that names the room the member is in: a `VoiceState` or a
  `DeviceAnnounce` for that room, or live media from them in our own call.
  This ends seats we learned without their device (a snapshot's hearsay) and
  seats whose leave we missed, for example while our app was restarting,
  even when the member stays online.

"Heard" means any live signed word from the member (presence beat, seat
news, typing and so on) or live media from them in our own call. The voice
device repeats its `DeviceAnnounce` and `VoiceState` every `BEAT` (45
seconds), also while appearing offline, so a live seat never times out.
Quitting the desktop app sends the leave messages before `Offline`.

## Audio links follow seats

An app that quits without saying goodbye (an update install, a crash) leaves
the others holding a dead audio link to it for a while. A link must never
stop a fresh call from forming:

- when the device a link goes to says it is not in our room, the link ends;
- when it announces a new join (a different `since`) and the link is older
  than a few seconds, the link belonged to its last session and ends, so the
  dialer calls the new one;
- every few seconds, links to members no longer seated in our room end.

## Late joiners

- Every device's greeting (`here`) carries its `DeviceAnnounce` and, when in
  voice, its `VoiceState`. Members answer a newcomer with their greeting.
- A server snapshot's voice list is the sender's hearsay. It only fills seats
  the receiver does not already know. Seats nobody then hears from end
  through the liveness rules above.
- Rebuilding a server from a snapshot keeps the live seats and the device
  registry (`ServerState::carry_live_voice`), including our own seat.

## One seat per person

Seats are keyed by account, never by device, so a person shows once in a
channel however many devices they have. Dial their media through
`ServerState::voice_node`, never through `profile.node`.

## Tests that pin these rules

- `commy-core` `devices`: a later join takes the seat, an older join heard
  late does not, a tie keeps the holder, a join stamp beats the seat it takes
  over, standing up clears every seat.
- `commy-core` `store`: only the device in voice can end the seat, the device
  in voice decides the room whatever its clock says, members without devices,
  ending voice, a fresh copy keeps who is in voice.
- `commy-view` `liveness`: a quiet voice device, a seat that keeps beating,
  a seat nobody confirms, news about another room, someone appearing
  offline, the sync service window, going offline on the voice device and a
  stale goodbye, waking from sleep.
- `commy-web-client` `client`: the browser's greeting seats it for late
  joiners, a bare leave does not unseat the voice device.
