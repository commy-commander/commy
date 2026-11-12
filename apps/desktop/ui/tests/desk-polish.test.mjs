import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = name => readFileSync(new URL(`../${name}`, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("index.html");
const app = read("scripts/app.js");
const base = read("styles/styles.css");
const studio = read("styles/studio.css");
const settings = read("styles/studio-settings.css");
const float = read("styles/studio-float.css");
const profile = read("styles/studio-profile.css");
const STUDIO = 'html[data-look="studio"]';
const DESK = `${STUDIO}:not([data-fit="slim"])`;

test("a channel's banner sits at the top of the chat on desktop and the welcome block drops its copy", () => {
  assert.ok(html.includes('<div class="chat-banner" id="chat-banner" aria-hidden="true"></div>\n      <div class="messages" id="messages"></div>'));
  assert.ok(base.includes(".chat-banner { display: none; }"));
  assert.ok(studio.includes(`${DESK} .main.has-chat-banner > .chat-banner {`));
  assert.ok(studio.includes(`${DESK} .main.has-chat-banner > .chat-banner:has(+ .messages.hidden) { display: none; }`));
  assert.ok(studio.includes(`${DESK} .channel-intro-banner { display: none; }`));
  assert.match(app, /function paintDeskChatBanner\(\) \{\n  const node = \$\("chat-banner"\);\n  const ref = chatBanner\(\{ view: state\.view, channel: state\.currentChannel \}\);/);
  assert.ok(app.includes('new MutationObserver(paintDeskChatBanner).observe($("channel-name"), { childList: true, characterData: true, subtree: true });'));
});

test("the fixed AFK section keeps its capitals in the channel list", () => {
  assert.ok(app.includes('section.fixed ? "section-title keep-case" : "section-title"'));
  assert.ok(studio.includes(`${DESK} .section-title.keep-case { text-transform: none; }`));
});

test("a slider in a settings row leaves the words their room, and a kind picker beside a name keeps its width", () => {
  assert.ok(settings.includes(`${STUDIO} .settings-layer .setting-row:has(> input[type="range"]) { flex-wrap: wrap; row-gap: 8px; }`));
  assert.ok(settings.includes(`${STUDIO} .settings-layer .setting-row:has(> input[type="range"]) > div:first-child { flex: 3 1 260px; }`));
  assert.ok(settings.includes(`${STUDIO} .settings-layer .setting-row > input[type="range"] { flex: 1 1 240px; width: auto; min-width: 0; }`));
  assert.ok(settings.includes(`${DESK} .settings-layer .setting-row > input[type="range"] { max-width: 280px; }`));
  assert.ok(settings.includes(`${STUDIO} .inline-row > .set-pick { flex: 0 0 auto; min-width: 128px; }`));
  assert.match(html, /<div class="settings-card setting-row hidden" id="channel-limit-row">[\s\S]*?<input type="range" id="channel-limit" min="0" max="99" value="0" \/>\n\s+<\/div>/);
});

test("the games search list has its own backing and sits above the next card", () => {
  assert.ok(settings.includes(`${STUDIO} .set-pane:has(.game-search:not(.hidden)) { z-index: 5; }`));
  const rule = settings.slice(settings.indexOf(`${DESK} .game-search {`));
  assert.match(rule.slice(0, rule.indexOf("}")), /background: linear-gradient\(180deg, rgb\(var\(--lamp\) \/ \.06\), rgb\(var\(--lamp\) \/ 0\) 60%\), var\(--bg-raised\);/);
});

test("the header keys sit on one line and GIF is the last key in the message bar", () => {
  assert.ok(studio.includes(`${DESK} .header-actions > #threads-btn > svg { translate: 0 -1px; }`));
  assert.ok(studio.includes(`${DESK} .header-actions > #pins-btn > svg { translate: -.5px .5px; }`));
  assert.ok(studio.includes(`${DESK} .composer #gif-btn { order: 1; margin-right: 8px; }`));
  assert.ok(studio.includes(`${DESK} .composer #gif-btn:not(.hidden) + #emoji-btn { margin-right: 0; }`));
  assert.ok(html.indexOf('id="text-style-btn"') < html.indexOf('id="gif-btn"') && html.indexOf('id="gif-btn"') < html.indexOf('id="emoji-btn"'));
});

test("searching settings hides the tabs so the results show, and camera words find the camera", () => {
  assert.ok(settings.includes(`${DESK} .settings-nav-inner.searching > :is(.settings-tab, .settings-nav-group, .settings-nav-sep) { display: none; }`));
  assert.ok(settings.indexOf(`${DESK} .settings-nav-inner.searching >`) < settings.indexOf(`${DESK} .settings-tab {\n  display: flex;`));
  assert.ok(html.includes('id="cam-bg-label" data-search="blur image backdrop virtual"'));
  assert.ok(html.includes('id="cam-avatar-label" data-search="vrm face cat robot alien"'));
});

test("game tags under the server name and in server info open the game card, and the editor chips stay plain", () => {
  assert.equal(app.split('gameTagKey(server, tag, "game-chip small")').length - 1, 2);
  assert.ok(app.includes('const chip = gameChip(state.currentServer, tag, "game-chip");'));
  assert.ok(html.includes('<div class="game-pop hidden" id="game-pop" role="dialog" aria-label="Game"></div>'));
  assert.match(app, /function fillGamePop\(name, details, cover, looking\) \{[\s\S]*?const steam = details && steamButton\(details\.steam\);/);
  assert.ok(app.includes('"text-style-pop", "game-pop"]) {'));
  assert.ok(app.includes('closeMenuOutside("game-pop", e.target, ".game-link");'));
  assert.ok(app.includes("gameLink(card, game, art, () => cover);"));
  assert.ok(base.includes(".game-link { cursor: pointer; }"));
  assert.ok(profile.includes(`${STUDIO} .game-pop .ac-steam {`));
  assert.ok(float.includes(`${DESK} .sidebar-game-tags .game-chip:hover { background: var(--bg-hover); color: var(--header-primary); }`));
});

test("an owner who leaves is sent to hand the server over first, then to the leave question", () => {
  assert.match(app, /if \(full && state\.user && full\.owner === state\.user\.id && heirsOf\(full\)\.length\) \{\n\s+openTransferOwner\(full, true\);\n\s+return;/);
  assert.match(app, /if \(!server\.group\) \{\n\s+const full = await serverToLeave\(serverId\);/);
  assert.ok(app.includes("if (leaving) await openLeaveServer(server.id);"));
  assert.ok(app.includes('leave: ["Hand the server over first",'));
  assert.ok(html.includes('<h2 id="transfer-owner-title">Transfer Ownership</h2>'));
  assert.ok(html.includes('<p id="transfer-owner-body">'));
});

test("the profile bar is as wide as the member list and holds its place while direct messages load", () => {
  assert.ok(studio.includes(`${DESK} .dm-profile { grid-column: 3; width: var(--st-wing); }`));
  assert.match(app, /async function enterDmMode\(openFirst = true\) \{\n  setViewMode\("dm"\);\n  if \(openFirst && state\.showDmProfile && studioDesk\(\)\) \$\("dm-profile"\)\.classList\.remove\("hidden"\);/);
});
