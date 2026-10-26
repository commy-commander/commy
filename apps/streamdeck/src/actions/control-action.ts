import streamDeck, {
  type Action,
  type DialRotateEvent,
  type DidReceiveSettingsEvent,
  type KeyDownEvent,
  type KeyUpEvent,
  type SendToPluginEvent,
  SingletonAction,
  type WillAppearEvent,
  type WillDisappearEvent,
} from "@elgato/streamdeck";

import type { CommyLink } from "../commy-link.ts";
import {
  actionId,
  type ControlKind,
  type ControlSettings,
  dialControl,
  keyLook,
  pressControl,
  releaseControl,
  volumeOf,
} from "../controls.ts";
import type { Control, DeckStatus } from "../protocol.ts";
import { stateMessage } from "../sound-keys.ts";
import type { StatusFeed } from "../status-feed.ts";

type InspectorMessage = { event: "refresh" };

export class ControlAction extends SingletonAction<ControlSettings> {
  override readonly manifestId: string;
  #kind: ControlKind;
  #link: CommyLink;
  #feed: StatusFeed;
  #settings = new Map<string, ControlSettings>();
  #shown = new Map<string, string>();
  #stopWatching: (() => void) | null = null;

  constructor(kind: ControlKind, link: CommyLink, feed: StatusFeed) {
    super();
    this.manifestId = actionId(kind);
    this.#kind = kind;
    this.#link = link;
    this.#feed = feed;
  }

  override async onWillAppear(ev: WillAppearEvent<ControlSettings>): Promise<void> {
    this.#settings.set(ev.action.id, ev.payload.settings);
    this.#shown.delete(ev.action.id);
    this.#stopWatching ??= this.#feed.watch(status => void this.#paintAll(status));
    await this.#paint(ev.action, this.#feed.last);
  }

  override onWillDisappear(ev: WillDisappearEvent<ControlSettings>): void {
    this.#settings.delete(ev.action.id);
    this.#shown.delete(ev.action.id);
    if (this.#settings.size === 0) {
      this.#stopWatching?.();
      this.#stopWatching = null;
    }
  }

  override async onDidReceiveSettings(ev: DidReceiveSettingsEvent<ControlSettings>): Promise<void> {
    this.#settings.set(ev.action.id, ev.payload.settings);
    this.#shown.delete(ev.action.id);
    await this.#paint(ev.action, this.#feed.last);
  }

  override async onKeyDown(ev: KeyDownEvent<ControlSettings>): Promise<void> {
    const control = pressControl(this.#kind, ev.payload.settings);
    if (!control) {
      await ev.action.showAlert();
      return;
    }
    if (!(await this.#send(control))) await ev.action.showAlert();
  }

  override async onKeyUp(ev: KeyUpEvent<ControlSettings>): Promise<void> {
    const control = releaseControl(this.#kind);
    if (control) await this.#send(control);
  }

  override async onDialRotate(ev: DialRotateEvent<ControlSettings>): Promise<void> {
    const control = dialControl(this.#kind, ev.payload.settings, ev.payload.ticks);
    if (control) await this.#send(control);
  }

  override async onSendToPlugin(ev: SendToPluginEvent<InspectorMessage, ControlSettings>): Promise<void> {
    if (ev.payload?.event !== "refresh") return;
    const state = await this.#link.connect();
    if (state !== "ready") {
      await streamDeck.ui.sendToPropertyInspector({ event: "directory", state, message: stateMessage(state) });
      return;
    }
    try {
      const directory = await this.#link.directory();
      await streamDeck.ui.sendToPropertyInspector({ event: "directory", state, message: "", directory, status: this.#feed.last });
    } catch {
      await streamDeck.ui.sendToPropertyInspector({ event: "directory", state: "offline", message: stateMessage("offline") });
    }
  }

  async #send(control: Control): Promise<boolean> {
    const state = await this.#link.connect();
    if (state !== "ready") return state === "waiting";
    try {
      this.#feed.publish(await this.#link.control(control));
      return true;
    } catch (error) {
      streamDeck.logger.info(`${this.#kind} not done: ${String(error)}`);
      return false;
    }
  }

  async #paintAll(status: DeckStatus | null): Promise<void> {
    for (const action of this.actions) await this.#paint(action, status);
  }

  async #paint(action: Action<ControlSettings>, status: DeckStatus | null): Promise<void> {
    const settings = this.#settings.get(action.id) ?? {};
    const look = keyLook(this.#kind, settings, status);
    const volume = status ? volumeOf(this.#kind, settings, status) : null;
    const key = JSON.stringify([look, volume]);
    if (this.#shown.get(action.id) === key) return;
    this.#shown.set(action.id, key);
    if (action.isKey()) {
      await action.setState(look.state);
      await action.setTitle(look.title);
    } else if (action.isDial()) {
      const shown = volume === null ? "" : `${Math.round(volume * 100)}%`;
      await action.setFeedback({ title: look.title.split("\n")[0], value: shown, indicator: { value: volume === null ? 0 : volume * 50 } });
    }
  }
}
