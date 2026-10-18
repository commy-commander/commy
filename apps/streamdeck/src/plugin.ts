import streamDeck from "@elgato/streamdeck";

import { PlaySound } from "./actions/play-sound.ts";
import { CommyLink } from "./commy-link.ts";
import { globalTokenStore } from "./token-store.ts";

const link = new CommyLink({ tokens: globalTokenStore, name: "Stream Deck" });

streamDeck.actions.registerAction(new PlaySound(link));
streamDeck.connect();
