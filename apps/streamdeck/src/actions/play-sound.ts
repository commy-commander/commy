import streamDeck, {
  action,
  type DidReceiveSettingsEvent,
  type KeyDownEvent,
  type SendToPluginEvent,
  SingletonAction,
  type WillAppearEvent,
} from "@elgato/streamdeck";

import type { CommyLink, LinkState } from "../commy-link.ts";
import { keyTitle, playTarget, stateMessage, type SoundKeySettings } from "../sound-keys.ts";

type InspectorMessage = { event: string };

@action({ UUID: "chat.commy.soundboard.play" })
export class PlaySound extends SingletonAction<SoundKeySettings> {
  #link: CommyLink;

  constructor(link: CommyLink) {
    super();
    this.#link = link;
    link.onState(state => void this.#sendSoundboards(state));
  }

  override async onWillAppear(ev: WillAppearEvent<SoundKeySettings>): Promise<void> {
    if (ev.action.isKey()) await ev.action.setTitle(keyTitle(ev.payload.settings));
  }

  override async onDidReceiveSettings(ev: DidReceiveSettingsEvent<SoundKeySettings>): Promise<void> {
    if (ev.action.isKey()) await ev.action.setTitle(keyTitle(ev.payload.settings));
  }

  override async onKeyDown(ev: KeyDownEvent<SoundKeySettings>): Promise<void> {
    const target = playTarget(ev.payload.settings);
    if (!target) {
      await ev.action.showAlert();
      return;
    }
    const state = await this.#link.connect();
    if (state === "waiting") return;
    if (state !== "ready") {
      await ev.action.showAlert();
      return;
    }
    try {
      await this.#link.play(target.sound, target.server);
      await ev.action.showOk();
    } catch (error) {
      streamDeck.logger.info(`sound not played: ${String(error)}`);
      await ev.action.showAlert();
    }
  }

  override async onSendToPlugin(ev: SendToPluginEvent<InspectorMessage, SoundKeySettings>): Promise<void> {
    if (ev.payload?.event !== "refresh") return;
    await this.#sendSoundboards(await this.#link.connect());
  }

  async #sendSoundboards(state: LinkState): Promise<void> {
    if (!streamDeck.ui.action) return;
    if (state !== "ready") {
      await streamDeck.ui.sendToPropertyInspector({ event: "soundboards", state, message: stateMessage(state) });
      return;
    }
    try {
      const list = await this.#link.list();
      await streamDeck.ui.sendToPropertyInspector({ event: "soundboards", state, message: "", list });
    } catch {
      await streamDeck.ui.sendToPropertyInspector({ event: "soundboards", state: "offline", message: stateMessage("offline") });
    }
  }
}
