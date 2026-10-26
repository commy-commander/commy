import streamDeck, {
  action,
  type DidReceiveSettingsEvent,
  type KeyDownEvent,
  type SendToPluginEvent,
  SingletonAction,
  type WillAppearEvent,
} from "@elgato/streamdeck";

import type { CommyLink, LinkState, NewSound } from "../commy-link.ts";
import { keyTitle, playTarget, stateMessage, type SoundKeySettings } from "../sound-keys.ts";
import { UploadAssembler, type UploadPart, type UploadStart } from "../uploads.ts";

type InspectorMessage =
  | { event: "refresh" }
  | ({ event: "upload-start" } & UploadStart)
  | ({ event: "upload-part" } & UploadPart);

type SoundAction = SendToPluginEvent<InspectorMessage, SoundKeySettings>["action"];

@action({ UUID: "chat.commy.soundboard.play" })
export class PlaySound extends SingletonAction<SoundKeySettings> {
  #link: CommyLink;
  #uploads = new UploadAssembler();

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
    const message = ev.payload;
    if (message?.event === "refresh") {
      await this.#sendSoundboards(await this.#link.connect());
    } else if (message?.event === "upload-start") {
      if (!this.#uploads.start(message)) await this.#sendUploadResult(message.upload, "That upload could not start, try again.");
    } else if (message?.event === "upload-part") {
      let sound: NewSound | null;
      try {
        sound = this.#uploads.add(message);
      } catch (error) {
        await this.#sendUploadResult(message.upload, error instanceof Error ? error.message : String(error));
        return;
      }
      if (sound) await this.#addSound(ev.action, message.upload, sound);
    }
  }

  async #addSound(target: SoundAction, upload: string, sound: NewSound): Promise<void> {
    const state = await this.#link.connect();
    if (state !== "ready") {
      await this.#sendUploadResult(upload, stateMessage(state));
      return;
    }
    try {
      const added = await this.#link.addSound(sound);
      const settings: SoundKeySettings = { server: sound.server, sound: added.name, label: added.label, emoji: added.emoji };
      await target.setSettings(settings);
      if (target.isKey()) await target.setTitle(keyTitle(settings));
      await this.#sendUploadResult(upload, "", settings);
      await this.#sendSoundboards(state);
    } catch (error) {
      await this.#sendUploadResult(upload, error instanceof Error ? error.message : String(error));
    }
  }

  async #sendUploadResult(upload: string, message: string, settings?: SoundKeySettings): Promise<void> {
    await streamDeck.ui.sendToPropertyInspector({ event: "upload-result", upload, ok: !message, message, settings: settings ?? null });
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
