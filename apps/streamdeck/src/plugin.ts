import streamDeck from "@elgato/streamdeck";

import { ControlAction } from "./actions/control-action.ts";
import { PlaySound } from "./actions/play-sound.ts";
import { CommyLink } from "./commy-link.ts";
import { CONTROL_KINDS } from "./controls.ts";
import { StatusFeed } from "./status-feed.ts";
import { globalTokenStore } from "./token-store.ts";

const link = new CommyLink({ tokens: globalTokenStore, name: "Stream Deck" });
const feed = new StatusFeed(link);

streamDeck.actions.registerAction(new PlaySound(link));
for (const kind of CONTROL_KINDS) streamDeck.actions.registerAction(new ControlAction(kind, link, feed));
streamDeck.connect();
