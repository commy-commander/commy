"use strict";

const BUILTIN_EMOTICONS = [
  {
    group: "Happy",
    faces: [
      ["happy", "(◕‿◕)"],
      ["yay", "\\(^o^)/"],
      ["sparkle", "(ﾉ◕ヮ◕)ﾉ*:･ﾟ✧"],
      ["grin", "(^▽^)"],
      ["proud", "(￣▽￣)ゞ"],
      ["blush", "(⁄ ⁄•⁄ω⁄•⁄ ⁄)"],
      ["owo", "(・ω・)"],
      ["uwu", "(◡ ω ◡)"],
    ],
  },
  {
    group: "Love",
    faces: [
      ["love", "(♥ω♥*)"],
      ["hug", "(っ◕‿◕)っ"],
      ["big_hug", "(づ｡◕‿‿◕｡)づ"],
      ["kiss", "(˘³˘)♥"],
      ["heart", "(´∀｀)♡"],
    ],
  },
  {
    group: "Shrug and stare",
    faces: [
      ["shrug", "¯\\_(ツ)_/¯"],
      ["lenny", "( ͡° ͜ʖ ͡°)"],
      ["disapproval", "ಠ_ಠ"],
      ["cool", "(⌐■_■)"],
      ["side_eye", "(¬_¬)"],
      ["shocked", "(☉_☉)"],
      ["thinking", "(¬‿¬ )"],
      ["huh", "(・_・ヾ"],
    ],
  },
  {
    group: "Sad and angry",
    faces: [
      ["cry", "(╥﹏╥)"],
      ["sad", "(︶︹︶)"],
      ["sweat", "(^_^;)"],
      ["sigh", "(－‸ლ)"],
      ["angry", "(╬ Ò﹏Ó)"],
      ["rage", "ヽ(`Д´)ﾉ"],
    ],
  },
  {
    group: "Tables",
    faces: [
      ["table_flip", "(╯°□°)╯︵ ┻━┻"],
      ["table_back", "┬─┬ノ( º _ ºノ)"],
      ["double_flip", "┻━┻ ︵ヽ(`Д´)ﾉ︵ ┻━┻"],
    ],
  },
  {
    group: "Animals",
    faces: [
      ["cat", "(=^･ω･^=)"],
      ["bear", "ʕ•ᴥ•ʔ"],
      ["dog", "U・ᴥ・U"],
      ["fish", "<°)))><"],
    ],
  },
  {
    group: "Actions",
    faces: [
      ["wave", "(・ω・)ノ"],
      ["dance", "♪┏(・o･)┛♪"],
      ["flex", "ᕦ(ò_óˇ)ᕤ"],
      ["run", "ε=ε=┌( >_<)┘"],
      ["sleep", "(－_－) zzZ"],
      ["fight", "(ง •̀_•́)ง"],
      ["gg", "(•̀ᴗ•́)و"],
    ],
  },
];

const STICKER_TOKEN_RE = /^<sticker:([a-z0-9_]{2,32}):([A-Za-z0-9]{1,128}@[A-Za-z0-9]{1,128})>$/;

function parseStickerToken(text) {
  const m = STICKER_TOKEN_RE.exec(String(text || "").trim());
  return m ? { name: m[1], image: m[2] } : null;
}

function stickerToken(name, image) {
  return `<sticker:${name}:${image}>`;
}

function builtinEmoticons() {
  return BUILTIN_EMOTICONS.flatMap(({ faces }) => faces.map(([name, text]) => ({ name, text })));
}

function expressionMatches(items, query, label) {
  const q = String(query || "").trim().toLowerCase();
  return items.filter(item => !q || item.name.includes(q) || (label && String(item[label] || "").toLowerCase().includes(q)));
}

function pickerGroups(here, shared, kind, { external = true } = {}) {
  const groups = [];
  if (here && (here[kind] || []).length) groups.push({ id: here.id, name: here.name, items: here[kind], local: true });
  if (!external) return groups;
  const others = new Map();
  for (const item of shared || []) {
    if (here && item.server === here.id) continue;
    if (!others.has(item.server)) others.set(item.server, { id: item.server, name: item.server_name, items: [], local: false });
    const { server, server_name, ...rest } = item;
    others.get(item.server).items.push(rest);
  }
  return groups.concat([...others.values()]);
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { BUILTIN_EMOTICONS, parseStickerToken, stickerToken, builtinEmoticons, expressionMatches, pickerGroups };
}
