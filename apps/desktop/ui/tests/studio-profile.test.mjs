import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { PROFILE_WASH, PROFILE_FRIEND_KEYS, profileHasBanner, profileSubParts, profileStatusParts, profileFriendKey, profileMenuRows, profilePaneRows } = require("../scripts/studio-profile.js");
const { STUDIO_SPARKLES, STUDIO_DUST, STUDIO_SNOW, STUDIO_BUBBLES, STUDIO_CURTAINS, STUDIO_FIREFLIES, STUDIO_RAIN, STUDIO_NIGHT, STUDIO_METEORS, STUDIO_PETALS, STUDIO_MOTES, STUDIO_CLOUDS, STUDIO_CONFETTI, studioOrnaments, studioFrame, studioSnow, studioCurtain, studioEffect } = require("../scripts/profile-look.js");
const read = path => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const html = read("../index.html");
const css = read("../styles/studio-profile.css");
const app = read("../scripts/app.js");

const SWITCH = 'html[data-look="studio"]';
const FRAMES = /@keyframes[^{]+\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}/g;
const selectors = sheet => [...sheet.replace(FRAMES, "").replace(/@(media|container)[^{]+\{/g, "").matchAll(/(^|\})\s*([^{}@]+)\{/g)].flatMap(match => match[2].split(",").map(part => part.trim())).filter(Boolean);
const rule = selector => {
  const alone = css.indexOf(`${SWITCH} ${selector} {`);
  const start = alone >= 0 ? alone : css.indexOf(`${SWITCH} ${selector},\n`);
  assert.ok(start >= 0, selector);
  return css.slice(start, css.indexOf("}", start));
};

test("the profile sheet stays off until the new look is on, and takes its colours from the tokens", () => {
  const all = selectors(css);
  assert.ok(all.length > 100);
  for (const selector of all) assert.ok(selector.startsWith(SWITCH), selector);
  assert.equal(css.includes("/*"), false);
  assert.deepEqual([...new Set(css.match(/#[0-9a-fA-F]{3,8}\b/g) || [])], ["#000"]);
  assert.doesNotMatch(css, /rgba?\(\s*\d/);
  const desktop = html.indexOf('id="studio-desktop"');
  const profile = html.indexOf('<link rel="stylesheet" id="studio-profile" href="styles/studio-profile.css" media="not all" />');
  assert.ok(desktop > 0 && profile > desktop && html.indexOf('id="studio-phone"') > profile);
  assert.ok(html.indexOf('<script src="scripts/studio-profile.js"></script>') < html.indexOf('<script src="scripts/app.js"></script>'));
});

test("the card is 352 wide on smoked glass, with the banner under it fading out from half way", () => {
  const card = rule(".member-card");
  for (const line of ["width: 352px;", "padding: 0 0 16px;", "border-radius: var(--radius-xl);"]) assert.ok(card.includes(line), line);
  assert.match(app, /card\.classList\.toggle\("slab", studio\);/);
  const banner = rule(".mc-banner");
  assert.ok(banner.includes("height: 108px;"));
  assert.ok(banner.includes("mask-image: linear-gradient(#000 52%, transparent);"));
  const wash = rule(".mc-banner.wash");
  assert.ok(wash.includes("height: 84px;"));
  assert.ok(wash.includes("color-mix(in srgb, var(--pc) 46%, transparent), color-mix(in srgb, var(--pc) 12%, transparent)"));
  assert.ok(PROFILE_WASH.includes("var(--pc) 46%") && PROFILE_WASH.includes("var(--pc) 12%"));
  assert.match(app, /banner\.classList\.toggle\("wash", bare\);\n  if \(studio\) banner\.style\.setProperty\("--pc", member\.color \? `#\$\{member\.color\}` : personTint\(member\.id\)\);/);
  assert.equal(profileHasBanner({ banner: "blob:abc" }), true);
  assert.equal(profileHasBanner({ role_banner: "ref" }), true);
  assert.equal(profileHasBanner({ banner: "emoji:x" }), false);
  assert.equal(profileHasBanner({}), false);
  assert.equal(profileHasBanner(null), false);
});

test("the avatar is 84 round on the banner's fade with the keys on its line", () => {
  assert.ok(rule(".mc-head").includes("margin-top: -50px;"));
  const avatar = rule(".mc-head .avatar");
  for (const line of ["width: 84px;", "height: 84px;", "border: 0;"]) assert.ok(avatar.includes(line), line);
  assert.ok(rule(".mc-head .presence-dot").includes("box-shadow: 0 0 0 3.5px var(--bg-dark);"));
  assert.ok(rule(".mc-head .mc-actions").includes("margin: 0 0 0 auto;"));
  assert.ok(css.includes(`${SWITCH} .mc-actions .key.sq { width: 36px; height: 36px; }`));
  assert.match(app, /if \(studio\) card\.querySelector\("\.mc-head"\)\.appendChild\(actions\);\n  else \$\("mc-activity"\)\.after\(actions\);/);
  const more = rule(".member-card > .mc-more");
  for (const line of ["top: 10px;", "right: 10px;", "width: 32px;", "height: 32px;"]) assert.ok(more.includes(line), line);
  assert.match(app, /more\.className = studio \? "key sq mc-more theme-dark" : "icon-btn mc-more";/);
});

test("the keys line holds Message and one friend key, and your own card holds Edit profile", () => {
  assert.equal(profileFriendKey("none"), PROFILE_FRIEND_KEYS.none);
  assert.deepEqual(profileFriendKey("none"), { icon: "user-plus", title: "Add friend", cls: "key sq" });
  assert.deepEqual(profileFriendKey("outgoing"), { icon: "clock", title: "Request sent", cls: "key sq mc-sent" });
  assert.deepEqual(profileFriendKey("incoming"), { icon: "user-check", title: "Accept friend request", cls: "key sq primary mc-accept" });
  assert.equal(profileFriendKey("friend"), null);
  assert.equal(profileFriendKey("none", { bot: true }), null);
  assert.equal(profileFriendKey("incoming", { me: true }), null);
  assert.ok(css.includes(`${SWITCH} .mc-actions .key.mc-accept { --k: var(--accent-2); --k-ink: var(--accent-2-ink); }`));
  assert.ok(css.includes(`${SWITCH} .mc-actions .key.mc-sent { --k-ink: var(--text-muted); }`));
  assert.match(app, /key\("key sm", "pencil", "Edit profile", "", \(\) => \{\n        closeFloating\(\);\n        openUserSettings\("account"\);/);
  assert.match(app, /\} else if \(!member\.bot && !isBlocked\(member\.id\)\) \{\n      key\("key sm primary", "message-circle", "Message", "", \(\) => \{\n        closeFloating\(\);\n        messageUser\(member\.id, member\.name\);/);
});

test("More lists the profile, the invite and the friend rows first, and the red rows last", () => {
  assert.deepEqual(profileMenuRows({}), ["profile", "invite", "block"]);
  assert.deepEqual(profileMenuRows({ friend: true, reportable: true, canTimeOut: true }), ["profile", "invite", "notify", "unfriend", "block", "report", "timeout"]);
  assert.deepEqual(profileMenuRows({ bot: true, reportable: true }), ["profile", "block", "report"]);
  assert.deepEqual(profileMenuRows({ me: true }), []);
  assert.deepEqual(profileMenuRows({ me: true, canNickname: true }), ["nickname"]);
  assert.match(app, /more\.classList\.toggle\("hidden", studio \? !rows\.length : isMe\);/);
  assert.match(app, /openProfileMenu\(member, r\.right - 220, r\.bottom \+ 6, "member-card", studio \? \{ rows, anchor: more \} : null\);/);
  assert.match(app, /const shown = card \? card\.rows : \[friendState\(person\.id\) === "friend" \? "notify" : "", "block", targets\.length \? "report" : ""\];/);
  assert.match(app, /\}, card \? "danger" : ""\),\n    report:/);
});

test("under the name sit the username and pronouns, then the status with its emoji and never the game", () => {
  assert.deepEqual(profileSubParts({ username: "nova_owl", pronouns: "she/her" }), ["nova_owl", "she/her"]);
  assert.deepEqual(profileSubParts({ username: "mira" }), ["mira"]);
  assert.deepEqual(profileSubParts({ pronouns: " they/them " }), ["they/them"]);
  assert.deepEqual(profileSubParts({}), []);
  assert.deepEqual(profileSubParts(null), []);
  assert.deepEqual(profileStatusParts({ status_emoji: "🗺️", custom_status: "Mapping the lantern caves", activity: "Playing Second Orbit" }, ""), { emoji: "🗺️", words: "Mapping the lantern caves", shown: true });
  assert.deepEqual(profileStatusParts({ custom_status: "heads down" }, "Back at 5:00 PM"), { emoji: "", words: "Back at 5:00 PM · heads down", shown: true });
  assert.deepEqual(profileStatusParts({ activity: "Playing Second Orbit" }, ""), { emoji: "", words: "", shown: false });
  assert.deepEqual(profileStatusParts(null, ""), { emoji: "", words: "", shown: false });
  assert.match(app, /status\.classList\.toggle\("hidden", studio && !said\.shown\);/);
  assert.ok(rule(".mc-name").includes("font: 700 22px/26px var(--font-display);"));
  assert.ok(rule(".mc-name-row").includes("flex-wrap: wrap;"));
  assert.ok(html.includes('<div class="mc-label hidden" id="mc-about-label">About me</div>\n      <div class="mc-about hidden" id="mc-about"></div>'));
  assert.ok(html.includes('<div class="composer mc-composer mc-field" id="mc-composer">'));
});

test("role capsules are flat chips and the tags after the name share one quiet shape", () => {
  const chip = rule(".mc-role");
  for (const line of ["height: 26px;", "border-radius: 999px;", "background: var(--bg-active);", "font: 500 13px/1 var(--font);"]) assert.ok(chip.includes(line), line);
  assert.ok(!chip.includes("box-shadow"));
  assert.match(css, /\.mc-name-row \.timeout-badge \{\n  display: inline-flex;[^}]*color: var\(--text-muted\) !important;/);
  assert.ok(css.includes(`${SWITCH} .mc-name-row .timeout-badge::after { content: "Timed out"; }`));
  assert.match(app, /if \(member\.server_tag\) tags\.appendChild\(serverTagChip\(member\.server_tag\)\);\n    if \(member\.bot\) tags\.appendChild\(botTag\(\)\);\n    if \(member\.tag\) tags\.appendChild\(roleTagChip\(member\.tag, member\.tag_color\)\);\n    if \(timeoutActive\(member\.timeout_until\)\) tags\.appendChild\(timeoutBadge\(member\.timeout_until\)\);/);
});

test("an avatar border is a ring 2.5 thick that stands off the avatar, and none of them is red", () => {
  const ring = rule(".avatar > .avatar-border");
  for (const line of ["--bw: 2.5px;", "inset: -5px;", "--ring: var(--cream);"]) assert.ok(ring.includes(line), line);
  assert.ok(css.includes(`${SWITCH} .avatar > .ab-neon { --ring: var(--tx-pink);`));
  assert.ok(css.includes(`${SWITCH} .avatar > .ab-sunset { --ring: conic-gradient(var(--accent-2), var(--tx-orange), var(--tx-pink), var(--tx-purple), var(--accent-2)); }`));
  assert.doesNotMatch(css, /var\(--(red|danger)[a-z-]*\)/);
  const stars = studioOrnaments("stars");
  assert.equal(stars.length, 5);
  assert.deepEqual([stars[0].x, stars[0].y, stars[0].turn], [3.02, 32.9, 290]);
  assert.deepEqual([stars[1].x, stars[1].y, stars[1].turn], [21.32, 9.04, 325]);
  const leaves = studioOrnaments("leaves");
  assert.equal(leaves.length, 6);
  assert.deepEqual([leaves[0].x, leaves[0].y, leaves[0].turn, leaves[0].odd], [95.32, 71.13, 295, false]);
  assert.deepEqual([leaves[1].x, leaves[1].y, leaves[1].turn, leaves[1].odd], [81.47, 88.86, 321, true]);
  assert.deepEqual(studioOrnaments("ring"), []);
  assert.deepEqual(studioFrame("leaves").map(part => part.cls), ["ab-o", "ab-o b", "ab-o", "ab-o b", "ab-o", "ab-o b"]);
  assert.deepEqual(studioFrame("stars")[2], { cls: "ab-o", vars: { x: "50%", y: "0%", r: "360deg", z: 1.3, d: 1.4 }, path: stars[2].path });
  assert.match(app, /for \(const part of studioFrame\(id\)\) ring\.appendChild\(lookPart\(part, "0 0 10 10"\)\);/);
});

test("a ring is cut with a round mask, so it is the same thickness all the way round", () => {
  const before = rule(".avatar > .avatar-border::before");
  assert.ok(before.includes("mask: radial-gradient(closest-side, transparent calc(100% - var(--bw) - .4px), #000 calc(100% - var(--bw) + .4px));"));
  assert.ok(!before.includes("padding"));
});

test("a frame moves only on a big avatar or under the pointer, and stands still in a list", () => {
  assert.ok(rule(".avatar > .avatar-border").includes("--play: paused;"));
  assert.ok(css.includes(`${SWITCH} .avatar-lg > .avatar-border,\n${SWITCH} .avatar-xl > .avatar-border,\n${SWITCH} .mc-head .avatar > .avatar-border,\n${SWITCH} #pt-prof-av .avatar > .avatar-border,\n${SWITCH} .pt-profile .look.on .avatar > .avatar-border,\n${SWITCH} .look-option:hover .avatar > .avatar-border,\n${SWITCH} .look-option:focus-visible .avatar > .avatar-border { --play: running; }`));
  const frames = css.slice(css.indexOf(`${SWITCH} .avatar.has-border`), css.indexOf(`${SWITCH} .card-effect.fx {`));
  const moving = frames.match(/animation: ab-[^;]*;/g);
  assert.equal(moving.length, 10);
  for (const line of moving) assert.ok(line.endsWith("var(--play);"), line);
  assert.ok(rule(".ab-art .di-hand").includes("animation: ab-turn 60s steps(60) infinite var(--play);"));
  assert.ok(rule(".ab-art .bl-wreath").includes("animation: ab-turn 120s linear infinite var(--play);"));
  assert.ok(rule(".ab-art .fr-core").includes("animation: ab-glint 4.2s ease-in-out infinite var(--play);"));
  assert.ok(css.includes(`${SWITCH} .ab-art g,\n${SWITCH} .ab-art path { transform-box: view-box; transform-origin: 50% 50%; }`));
  assert.doesNotMatch(frames, /--tx-red|--red|--danger/);
  assert.ok(css.includes(`${SWITCH} .avatar > .ab-halo { --ring: var(--tx-yellow); --bw: 2px; }`));
  assert.ok(css.includes(`${SWITCH} .avatar > .ab-ripple { --ring: var(--tx-teal); --bw: 2px; }`));
  for (const ink of ["--accent-2", "--accent-lit", "--ring-on"]) assert.equal(frames.split("\n").filter(line => /ab-(halo|eclipse|ripple|wave)|\.(bl|fr|di)-/.test(line) && line.includes(ink)).length, 0, ink);
  assert.ok(rule(".ab-art .di-hand").includes("fill: var(--cream);"));
});

test("each profile effect is one fixed drawing, the same on every card", () => {
  assert.equal(STUDIO_SPARKLES.length, 9);
  assert.equal(STUDIO_SPARKLES.filter(star => star[4] === "hero").length, 1);
  assert.equal(STUDIO_SPARKLES.filter(star => star[4] === "sm").length, 5);
  assert.equal(STUDIO_DUST.length, 8);
  const sparkles = studioEffect("sparkles");
  assert.equal(sparkles.length, 17);
  assert.deepEqual(sparkles[0].vars, { x: 52, y: 34, z: 30, d: 0 });
  assert.equal(sparkles[0].cls, "s hero");
  assert.deepEqual(sparkles[10], { cls: "dust", vars: { x: 58, y: 44, d: 0.7 } });
  assert.deepEqual(STUDIO_SNOW.map(layer => [layer[0], layer[1]]), [["far", 12], ["mid", 9], ["near", 4]]);
  const snow = studioEffect("snow");
  assert.equal(snow.length, 25);
  assert.deepEqual(snow[0], { cls: "far", vars: { x: 2.7, y: 28.3, z: 3.2, t: 12, p: 0.351 } });
  assert.deepEqual(snow[2].vars, { x: 18.4, y: 67.2, z: 3.2, t: 12, p: 0.104 });
  assert.deepEqual(studioSnow(3, 11), studioSnow(3, 11));
  assert.equal(STUDIO_BUBBLES.length, 7);
  assert.deepEqual(studioEffect("bubbles")[0].vars, { x: 52, y: 42, z: 30, d: 0, t: 9 });
  assert.deepEqual(studioEffect("aurora"), []);
  assert.deepEqual(STUDIO_CURTAINS.map(curtain => [curtain.cls, curtain.ink]), [["c3", "--accent-lit"], ["c2", "--tx-teal"], ["c1", "--tx-green"]]);
  const rays = studioCurtain(STUDIO_CURTAINS[0]);
  assert.equal(rays.length, 57);
  assert.deepEqual(rays.slice(0, 3), [{ x: -4, y: 0.62, h: 14.91, o: 0 }, { x: -2.9, y: 2.07, h: 14.16, o: 0.13 }, { x: -1.8, y: -1.76, h: 18.69, o: 0.08 }]);
  assert.match(app, /function paintCardEffect\(banner, effect\) \{\n  banner\.querySelector\(":scope > \.card-effect"\)\?\.remove\(\);\n  if \(isCardEffect\(effect\)\) banner\.appendChild\(studioEffectNode\(effect\)\);\n\}/);
  assert.ok(css.includes(`${SWITCH} .card-effect.fx {`));
  assert.ok(rule(".card-effect.fx").includes("container-type: size;"));
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{\n  html\[data-look="studio"\] \.card-effect\.fx i,\n  html\[data-look="studio"\] \.card-effect\.fx g \{ animation: none !important; \}/);
  assert.ok(css.includes(`${SWITCH} .look-option .card-effect.fx g { animation-play-state: paused; }`));
});

test("the seven new effects are fixed drawings too, with the light gathered on the right", () => {
  assert.equal(STUDIO_FIREFLIES.length, 10);
  assert.ok(STUDIO_FIREFLIES.filter(fly => fly[0] > 50).length >= 6);
  assert.deepEqual(studioEffect("fireflies")[1], { cls: "b", vars: { x: 62, y: 40, z: 6.5, d: 2.1, t: 11 } });
  assert.deepEqual(STUDIO_RAIN.map(layer => [layer[0], layer[1]]), [["far", 14], ["mid", 11], ["near", 6]]);
  assert.equal(studioEffect("rain").length, 31);
  const meteors = studioEffect("meteors");
  assert.equal(meteors.filter(part => part.cls === "m").length, STUDIO_METEORS.length);
  assert.equal(meteors.filter(part => part.cls.startsWith("dust")).length, STUDIO_NIGHT.length);
  assert.ok(STUDIO_METEORS.every(([x, y]) => x >= 40 && y <= 24));
  assert.deepEqual(STUDIO_PETALS.map(layer => [layer[0], layer[1]]), [["far", 5], ["mid", 5], ["near", 2]]);
  assert.ok(studioEffect("petals").every(part => part.path));
  assert.deepEqual(studioEffect("sunbeams").slice(0, 2), [{ cls: "beam" }, { cls: "beam b" }]);
  assert.equal(studioEffect("sunbeams").length, 2 + STUDIO_MOTES.length);
  assert.deepEqual(STUDIO_CLOUDS.map(cloud => cloud[0]), ["n1", "n2", "n3", "n4"]);
  const confetti = studioEffect("confetti");
  assert.equal(confetti.length, STUDIO_CONFETTI.count);
  assert.deepEqual([...new Set(confetti.map(bit => bit.cls))].sort(), ["k1", "k2", "k3", "k4", "k5", "k6"]);
  const effects = css.slice(css.indexOf(`${SWITCH} .fx .dust.lg`), css.indexOf("@keyframes fx-breathe"));
  assert.doesNotMatch(effects, /--tx-red|--red|--danger/);
  assert.ok(css.includes(`${SWITCH} .pt-profile .look:not(.on) .card-effect.fx i,\n${SWITCH} .pt-profile .look:not(.on) .card-effect.fx g,\n${SWITCH} .look-option .card-effect.fx i,\n${SWITCH} .look-option .card-effect.fx g { animation-play-state: paused; }`));
  assert.ok(css.includes(`  ${SWITCH} .fx.meteors .m { opacity: .9; }\n}`));
  assert.ok(css.includes(`${SWITCH}[data-reduced-motion] .fx.meteors .m { opacity: .9; }`));
});

test("the full profile and the DM side pane are built from the card's parts", () => {
  const rows = profileMenuRows({ friend: true, reportable: true, canTimeOut: true });
  assert.deepEqual(profilePaneRows(rows, "card"), rows);
  assert.deepEqual(profilePaneRows(rows, "full"), ["invite", "notify", "unfriend", "block", "report", "timeout"]);
  assert.deepEqual(profilePaneRows(rows, "dm"), ["invite", "notify", "unfriend", "block", "report", "timeout"]);
  assert.deepEqual(profilePaneRows(profileMenuRows({ me: true }), "full"), []);
  assert.deepEqual(profilePaneRows(rows, "phone"), ["invite", "notify", "unfriend", "block", "report", "timeout"]);
  assert.match(app, /\}\), studio && document\.documentElement\.dataset\.fit === "slim" \? "phone" : "card"\);/);
  assert.match(app, /openUserSettings\("account"\);\n      \}\)\.id = "mc-edit";/);
  const start = app.indexOf("function studioProfilePane(");
  const pane = app.slice(start, app.indexOf("\n}\n", start));
  for (const part of ['"mc-banner"', '"key sq mc-more theme-dark"', '"mc-head"', '"mc-actions"', '"mc-body"', '"mc-name-row"', '"mc-username"', '"mc-status"', '"play-box hidden"', '"mc-label", "About me"', '"mc-label", "Roles"', '"mc-label", "Mutual servers"', "mutualServerRows(servers, leave)"]) {
    assert.ok(pane.includes(part), part);
  }
  assert.match(pane, /place === "full" && !person\.bot && !isBlocked\(person\.id\)/);
  assert.doesNotMatch(pane, /Friendship|No servers in common|mc-field|composer/);
  const rowsStart = app.indexOf("function mutualServerRows(");
  const rowsFn = app.slice(rowsStart, app.indexOf("\n}\n", rowsStart));
  for (const part of ['"prof-server"', '"prof-tile"', "paintNarrowServerIcon(tile", '"prof-name"', "leave();"]) assert.ok(rowsFn.includes(part), part);
  assert.match(app, /modal\.appendChild\(studioProfilePane\(person, "full", \(\) => openFullProfile\(member\)\)\);/);
  assert.match(app, /\$\("dm-profile"\)\.appendChild\(studioProfilePane\(/);
  assert.match(css, /html\[data-look="studio"\] #modal-profile > :not\(\.profile-pane\),\nhtml\[data-look="studio"\] #dm-profile > :not\(\.profile-pane\) \{ display: none; \}/);
  assert.match(rule(".profile-pane > .mc-more"), /top: 10px;\n  right: 10px;\n  width: 32px;\n  height: 32px;/);
  assert.match(rule(".prof-tile"), /width: 32px;\n  height: 32px;[\s\S]*border-radius: 50%;/);
  assert.match(rule(".prof-server"), /font: 500 14px\/1\.2 var\(--font\);/);
});

test("on the card and the profile panes an idle dot is the moon and an offline dot is a ring", () => {
  assert.match(rule(".mc-head .presence-dot.idle"), /background: var\(--bg-dark\); box-shadow: 0 0 0 3\.5px var\(--bg-dark\), inset 5px -4\.5px 0 \.5px var\(--yellow\);/);
  assert.match(css, /html\[data-look="studio"\] \.mc-head \.presence-dot\.invisible \{ background: var\(--bg-dark\); box-shadow: 0 0 0 3\.5px var\(--bg-dark\), inset 0 0 0 3px var\(--text-faint\); \}/);
});

test("the card lists mutual servers after Roles in the new look", () => {
  assert.match(html, /<div class="mc-roles" id="mc-roles"><\/div>\n\s*<div class="mc-label hidden" id="mc-servers-label">Mutual servers<\/div>\n\s*<div class="prof-servers hidden" id="mc-servers"><\/div>/);
  assert.match(app, /const mutual = studio && !isMe \? mutualServersOf\(member\.id\) : \[\];/);
  assert.match(app, /\$\("mc-servers"\)\.replaceChildren\(\.\.\.mutualServerRows\(mutual, closeFloating\)\);/);
});

test("role coloured words keep the raw colour at Night and a deeper mix in Day", () => {
  const { roleInk } = require("../scripts/studio-profile.js");
  assert.equal(roleInk("", true), "");
  assert.equal(roleInk("6f9fe6", false), "#6f9fe6");
  assert.equal(roleInk("6f9fe6", true), "color-mix(in oklab, #6f9fe6 var(--role-ink-mix, 100%), var(--header-primary))");
  const parts = read("../styles/studio-parts.css");
  assert.match(parts, /html\[data-look="studio"\]\[data-theme="light"\] \{ --role-ink-mix: 55%; \}/);
  assert.match(parts, /html\[data-look="studio"\]\[data-theme="light"\] \.theme-dark \{ --role-ink-mix: 100%; \}/);
  assert.match(app, /node\.style\.color = color \? roleInk\(color, studioLook\(\)\) : "";/);
});
