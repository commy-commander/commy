import Link from "next/link";
import { headers } from "next/headers";
import { getLatestRelease } from "@/lib/releases";
import { detectPlatform, PLATFORM_NAMES } from "@/lib/release-assets";
import { REPO_URL } from "@/lib/site";

const features = [
  {
    title: "P2P voice",
    body: "Voice channels that connect peers directly over QUIC. Your audio never touches a company server.",
  },
  {
    title: "Noise cancellation",
    body: "One toggle. Background noise gone. No premium tier required to sound good.",
  },
  {
    title: "P2P file sharing",
    body: "Share files directly with your server members. Transfers stay isolated to your server.",
  },
  {
    title: "Screen sharing",
    body: "High quality screen and game sharing with no paywall, no resolution caps and no FPS tax.",
  },
  {
    title: "Soundboards, emojis, images",
    body: "All the fun parts included for everyone. Custom emojis and soundboards work in every server, free.",
  },
  {
    title: "Free and open source",
    body: "Written in Rust and licensed AGPL-3.0. Servers live on members' machines, so there is nothing to rent and nothing to shut down.",
  },
];

const steps = [
  {
    title: "Install Commy",
    body: "Download the app for your computer and pick a username. No account is needed to chat.",
  },
  {
    title: "Create a server",
    body: "Your server lives on your members' computers. Make channels, roles, rules and a soundboard.",
  },
  {
    title: "Share an invite link",
    body: "Send a commy.chat/i/ link. Friends click it, Commy opens, and they join you directly.",
  },
];

export default async function HomePage() {
  const platform = detectPlatform(headers().get("user-agent"));
  const release = await getLatestRelease();
  const direct = release?.downloads.find((d) => d.platform === platform && d.primary);

  return (
    <main>
      <section className="hero">
        <h1>Hang out with your people, not with a middleman</h1>
        <p>
          Commy is free, open source, peer-to-peer voice and text chat for
          communities, written in Rust. Voice, chat, file sharing, soundboards
          and screen sharing, connected directly between friends with iroh.
        </p>
        <div className="hero-actions">
          {direct ? (
            <a href={direct.url} className="btn btn-primary btn-large">
              Download for {PLATFORM_NAMES[direct.platform]}
            </a>
          ) : (
            <Link href="/download" className="btn btn-primary btn-large">
              Download Commy
            </Link>
          )}
          <a href={REPO_URL} className="btn btn-large" target="_blank" rel="noreferrer">
            View on GitHub
          </a>
        </div>
        <p className="muted hero-note">
          {release ? `${release.name} for Windows, macOS and Linux.` : "Windows, macOS and Linux."}{" "}
          <Link href="/download">All downloads</Link> · <Link href="/docs/getting-started">Read the docs</Link>
        </p>
      </section>

      <section className="hero-shot">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src="/screenshots/app-chat.png"
          width={1440}
          height={900}
          alt="The Commy desktop app showing a server with text channels, a voice channel and a member list"
        />
      </section>

      <section className="features">
        {features.map((f) => (
          <div className="feature-card" key={f.title}>
            <h3>{f.title}</h3>
            <p>{f.body}</p>
          </div>
        ))}
      </section>

      <section className="how">
        <h2>Up and running in a minute</h2>
        <ol>
          {steps.map((s) => (
            <li key={s.title}>
              <h3>{s.title}</h3>
              <p className="muted">{s.body}</p>
            </li>
          ))}
        </ol>
      </section>
    </main>
  );
}
