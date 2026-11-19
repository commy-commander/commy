# What Commy can do

- **Servers and channels.** Create a server from scratch or a starter
  template, add text and voice channels, forum channels for posts with tags,
  group servers into folders, and invite people with a link, and list a
  public server in Discover so others can find it. A Ctrl+K quick switcher
  jumps to any channel, conversation or server by name. Mark a channel age-restricted and it asks people to confirm
  they are 18 or older before showing anything. Copy any server's channels,
  roles and rules as a template code so others can start from the same
  layout, and hand a server you own to another member. Onboarding questions
  ask new members what they are here for and hand out the roles, and so the
  channels, that match; answers can be changed later.
- **Roles and moderation.** Colored roles with fine-grained permissions,
  down to who may embed links, read message history, start threads, post
  polls or schedule events, kick, ban and timeouts, per-server nicknames, server mute and deafen,
  moving people between voice channels from their menu or by dragging them,
  pinned messages and announcements,
  and auto moderation that holds messages with blocked words, links, too many
  mentions or flooding. A moderation bot you run yourself can remove new ones
  for everyone ([crates/commy-bot](../crates/commy-bot/README.md)), and bots can
  register slash commands that members pick from the composer with option
  hints; using one posts a compact "used /name" line that the bot answers.
  Channels can be limited to roles. Messages sent in a private channel are
  encrypted so only those roles and admins can read them.
- **Rich chat.** Replies, reactions, edits, named threads that can be
  archived or locked, emoji (including custom server emoji), images and
  files shown inline, voice messages recorded from the composer and played
  back in place, `/tts` messages that readers can have read aloud, clickable
  links, polls with a closing time, and a GIF picker when the server operator
  has set up a GIF provider key. A server owner can connect file storage
  on their own Cloudflare account so shared files load even while everyone
  who has them is offline ([apps/file-store](../apps/file-store/README.md)).
- **Message search.** Search a server by keyword, phrase, author, mention or
  channel, or your direct messages and groups, scoped to the conversation you
  are viewing or all of them. Search runs on your device over the history it
  keeps, pages through results fifty at a time newest first, and a result
  jumps straight to the message, even weeks back. Save any message to a
  per-device Saved list, or ask for a reminder about it in 20 minutes, an
  hour, three hours, tomorrow morning or at a time you pick; the reminder
  shows a notification when it is due.
- **Voice.** Opus voice with one-click noise cancellation and echo
  cancellation toggles, per-person volume, shared mute and deafen, priority
  speakers who turn everyone else down while they talk, a status line per
  voice channel, a soundboard with per-server clips, and voice filters
  (higher, lower, robot, radio, cave and whisper) that a server can switch
  off for some roles or everyone. Invite a friend or
  a channel to a call with a card that shows who is in it right now and a
  Join button. Stage channels seat an audience that only listens and can
  raise a hand; hosts invite people up to speak. Activities in a
  voice channel: watch a direct video link or an attached video together,
  kept within a second of each other, or play tic-tac-toe while the rest of
  the channel watches.
- **Screen and game streaming.** Share a whole monitor or a single window with
  a Sharp, Balanced or Smooth quality preset. Windows uses Windows Graphics
  Capture, so games stream cleanly, and can send what your computer plays
  along with the picture; viewers set their own stream volume.
- **Direct messages.** Private conversations between two people, delivered
  straight to their devices, with offline messages that arrive when the other
  person comes back. Pin the conversations you use most, and the channels too,
  so they stay at the top of the list on that device.
- **Presence and notifications.** Status, Do Not Disturb with a "back at"
  time, appear offline, notification toasts, and a notification level or a
  timed mute for each server and each channel. An optional toast when a
  friend comes online, for everyone or just the few you pick, quiet while
  you are Do Not Disturb. A red "NEW" line marks where
  you left off in a channel, any message can be marked unread, and read
  positions follow you between your devices in the same encrypted sync as
  your server list.
- **Game overlay.** On Windows, an in-game overlay shows who is talking and
  recent chat, in the screen corner you choose.
- **Settings you can find.** Settings are grouped into You, Voice & Video,
  Alerts & Sounds, App and Advanced, and a search box at the top of the list
  jumps straight to any setting.
- **Accessibility.** Settings > Accessibility has reduced motion (on by
  default when your system asks for it), a colour saturation slider for role
  and status colours, and high contrast. Settings > Appearance has a light
  theme or follow-system theme and a language picker. Settings, navigation and sign-in are translated
  into Spanish so far; the dictionaries live in
  `apps/desktop/ui/scripts/locales/`, one file per language, and the rest of
  the app can be translated the same way. Everything here is per device.
- **Safety tools.** Block and report people, an audit log of server changes,
  and invite links that expire or can be revoked. Choose who can see your
  profile details (everyone, friends and the servers you share, or friends
  only); your name and avatar always show. Friend requests can carry a short
  note so the other person knows who is asking.

Windows is the primary platform today. macOS and Linux builds are published
with each release; a few features, such as the game overlay and notification
toasts, are Windows only for now.
