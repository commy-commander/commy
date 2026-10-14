import Link from "next/link";
import { headers } from "next/headers";
import { getLatestRelease } from "@/lib/releases";
import { detectPlatform, PLATFORM_NAMES } from "@/lib/release-assets";
import Icon, { type IconName } from "./components/icons";
import Shot from "./components/shot";

type Feature = {
  id: string;
  icon: IconName;
  eyebrow: string;
  title: string;
  body: string;
  points: string[];
  shot: string;
  alt: string;
  tone: string;
};

const FEATURES: Feature[] = [
  {
    id: "chat",
    icon: "chat",
    eyebrow: "Text chat",
    title: "Conversations that feel alive",
    body: "Channels for every topic, with the little things that make a chat feel like a place: replies, threads, reactions, mentions, image previews and GIFs.",
    points: ["Replies and threads keep busy channels readable", "Custom emoji and reactions for every server", "Drop images and files straight into the chat"],
    shot: "/screenshots/chat.jpg",
    alt: "A Commy channel with replies, emoji reactions, an image, a shared file and an invite card",
    tone: "coral",
  },
  {
    id: "voice",
    icon: "mic",
    eyebrow: "Voice",
    title: "Hop in. Everyone sounds great.",
    body: "Click a voice channel and you are talking. Audio goes straight between members over an encrypted connection, encoded with Opus so it stays clear on any network.",
    points: ["Direct peer-to-peer routes, with a relay only as a fallback", "Live ping, packet loss and route for every person", "Per-person volume, plus mute and deafen in one click"],
    shot: "/screenshots/call.jpg",
    alt: "Commy connected to a voice channel, showing the voice connection panel with ping and a direct route per person",
    tone: "violet",
  },
  {
    id: "noise",
    icon: "wave",
    eyebrow: "Noise cancellation",
    title: "One switch. Background noise gone.",
    body: "Keyboards, fans and the dishwasher stay out of the call. Noise cancellation is a single toggle, free for everyone, with a mic test so you can hear the difference yourself.",
    points: ["Toggle it from settings or the voice panel", "Mic test plays you back through the full pipeline", "Input and output devices, volumes and idle timeout"],
    shot: "/screenshots/voice.jpg",
    alt: "Commy Voice and Audio settings with the Noise Cancellation toggle switched on",
    tone: "teal",
  },
  {
    id: "stream",
    icon: "screen",
    eyebrow: "Screen and game streaming",
    title: "Go live with your game in two clicks",
    body: "Share a whole screen or a single window with everyone in the voice channel. Pick Smooth for games, Sharp for code and documents, and watch together in focus or fullscreen.",
    points: ["Smooth, Balanced and Sharp quality presets", "Live resolution and frame rate for viewers", "Webcams and a game overlay come along too"],
    shot: "/screenshots/stream.jpg",
    alt: "Watching a member's live racing game stream inside a Commy voice channel",
    tone: "pink",
  },
  {
    id: "manage",
    icon: "crown",
    eyebrow: "Server management",
    title: "Run your server your way",
    body: "Roles with colors, tags and fine-grained permissions. Channels and categories you can drag into place, server rules, events, soundboards and moderation tools for when things get loud.",
    points: ["Roles members can pick for themselves", "Mute, kick, ban and member reports", "Events with RSVPs, rules gate, AFK channel"],
    shot: "/screenshots/roles.jpg",
    alt: "Commy server settings showing roles with colors, tags and permissions",
    tone: "amber",
  },
  {
    id: "invite",
    icon: "link",
    eyebrow: "Invite links",
    title: "Send a link. Your friends are in.",
    body: "Every server gets short commy.chat invite links. Friends click, Commy opens, and they join you directly. Links expire on their own and you can revoke them any time.",
    points: ["Short links that open the app", "A warning before joining from an unknown host", "Works with self-hosted account servers"],
    shot: "/screenshots/invite.jpg",
    alt: "Commy asking to confirm joining the Pixel Pioneers server from an invite link",
    tone: "blue",
  },
];

const EXTRAS: { icon: IconName; title: string; body: string }[] = [
  { icon: "file", title: "File sharing", body: "Up to 512 MB, sent member to member and kept inside your server." },
  { icon: "music", title: "Soundboards", body: "Upload clips and play them into voice." },
  { icon: "smile", title: "Custom emoji", body: "Your server's own emoji, in chat and reactions." },
  { icon: "calendar", title: "Events", body: "Plan game nights and see who is coming." },
  { icon: "users", title: "Friends and DMs", body: "Direct messages with message requests and blocking." },
  { icon: "screen", title: "Webcams", body: "Turn your camera on in any voice channel." },
  { icon: "spark", title: "Game overlay", body: "See who is talking without leaving your game." },
  { icon: "code", title: "Open source", body: "Rust, AGPL-3.0, built in the open." },
];

const STEPS = [
  { title: "Install Commy", body: "Download the app for Windows, macOS or Linux and sign in with a free account." },
  { title: "Create a server", body: "Add channels, roles, rules, emoji and a soundboard. It lives on your members' computers." },
  { title: "Share an invite link", body: "Friends click the link, Commy opens, and they join you directly." },
];

export default async function HomePage() {
  const platform = detectPlatform(headers().get("user-agent"));
  const release = await getLatestRelease();
  const direct = release?.downloads.find((d) => d.platform === platform && d.primary);

  const download = direct ? (
    <a href={direct.url} className="btn btn-primary btn-large btn-glow">
      <Icon name="download" size={20} /> Download for {PLATFORM_NAMES[direct.platform]}
    </a>
  ) : (
    <Link href="/download" className="btn btn-primary btn-large btn-glow">
      <Icon name="download" size={20} /> Download Commy
    </Link>
  );

  return (
    <main className="home">
      <section className="hero-x">
        <div className="hero-bg" aria-hidden="true">
          <span className="blob blob-a" />
          <span className="blob blob-b" />
          <span className="blob blob-c" />
          <span className="grid-floor" />
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img className="floaters" src="/art/floaters.svg" alt="" />
        </div>
        <div className="hero-copy">
          <p className="pill">
            <Icon name="heart" size={16} /> Free and open source, for everyone
          </p>
          <h1>
            Where your community <span className="grad">hangs out</span>
          </h1>
          <p className="lede">
            Commy is chat, voice and game streaming for your friends and communities. It runs directly between your
            members, so there is nothing to rent and no paywall to hit.
          </p>
          <div className="hero-actions">
            {download}
            <a href="#features" className="btn btn-large btn-ghost">
              See what it can do
            </a>
          </div>
          <p className="hero-note">
            {release ? `${release.name} for Windows, macOS and Linux.` : "For Windows, macOS and Linux."}{" "}
            <Link href="/download">All downloads</Link>
          </p>
        </div>
        <div className="hero-stage">
          <Shot src="/screenshots/chat.jpg" alt="The Commy app: a community server with channels, a voice channel in use and a busy chat" priority />
          <div className="chip chip-a">
            <Icon name="mic" size={18} />
            <span>
              Voice connected <em>19 ms, direct</em>
            </span>
          </div>
          <div className="chip chip-b">
            <span className="live">LIVE</span>
            <span>
              sprocket is streaming <em>Hoverlane Rally</em>
            </span>
          </div>
          <div className="chip chip-c">
            <Icon name="wave" size={18} />
            <span>Noise cancellation on</span>
          </div>
        </div>
      </section>

      <div className="marquee" aria-hidden="true">
        <div className="marquee-track">
          {[0, 1].map((n) => (
            <span key={n}>
              Text channels · Threads · Reactions · Custom emoji · GIFs · Voice channels · Noise cancellation · Screen
              sharing · Game streaming · Webcams · Roles · Permissions · Events · Soundboards · Invite links · File
              sharing · Friends and DMs · Game overlay ·{" "}
            </span>
          ))}
        </div>
      </div>

      <section id="features" className="features-x">
        {FEATURES.map((f, i) => (
          <article key={f.id} id={f.id} className={`feature-row tone-${f.tone} ${i % 2 ? "flip" : ""}`}>
            <div className="feature-copy" data-reveal>
              <p className="eyebrow">
                <span className="eyebrow-icon">
                  <Icon name={f.icon} size={18} />
                </span>
                {f.eyebrow}
              </p>
              <h2>{f.title}</h2>
              <p className="feature-body">{f.body}</p>
              <ul className="checks">
                {f.points.map((p) => (
                  <li key={p}>
                    <Icon name="check" size={18} />
                    {p}
                  </li>
                ))}
              </ul>
            </div>
            <div className="feature-shot" data-reveal>
              <Shot src={f.shot} alt={f.alt} />
            </div>
          </article>
        ))}
      </section>

      <section className="private">
        <div className="private-copy" data-reveal>
          <p className="eyebrow">
            <span className="eyebrow-icon">
              <Icon name="shield" size={18} />
            </span>
            Private by design
          </p>
          <h2>Your conversations never touch our servers</h2>
          <p className="feature-body">
            Messages, voice, video, streams and files travel directly between devices over encrypted connections. A free
            Commy account only proves you are a real person, which keeps spam bots out of your server.
          </p>
          <div className="private-points">
            <div>
              <strong>Peer to peer</strong>
              <span>No company server in the middle of your chats or calls.</span>
            </div>
            <div>
              <strong>No tracking</strong>
              <span>No ads, no analytics, no selling your data.</span>
            </div>
            <div>
              <strong>Host it yourself</strong>
              <span>
                Run your own account server with the <Link href="/docs/self-hosting">open source stack</Link>.
              </span>
            </div>
          </div>
        </div>
        <div className="feature-shot" data-reveal>
          <Shot src="/screenshots/signin.jpg" alt="Signing in to Commy with a one-time code shown in the app and approved in the browser" />
        </div>
      </section>

      <section className="extras">
        <h2 data-reveal>And everything else, included</h2>
        <p className="extras-sub" data-reveal>
          No premium tier. Every feature is free for every member of every server.
        </p>
        <div className="extras-grid">
          {EXTRAS.map((x) => (
            <div className="extra" key={x.title} data-reveal>
              <span className="extra-icon">
                <Icon name={x.icon} />
              </span>
              <h3>{x.title}</h3>
              <p>{x.body}</p>
            </div>
          ))}
        </div>
      </section>

      <section className="steps">
        <h2 data-reveal>Up and running in a minute</h2>
        <ol>
          {STEPS.map((s) => (
            <li key={s.title} data-reveal>
              <h3>{s.title}</h3>
              <p>{s.body}</p>
            </li>
          ))}
        </ol>
      </section>

      <section className="cta" data-reveal>
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img className="cta-logo" src="/logo.png" alt="" width={88} height={88} />
        <h2>Bring your people to Commy</h2>
        <p>Free forever, open source, and ready for Windows, macOS and Linux.</p>
        <div className="hero-actions">
          {download}
          <Link href="/docs/getting-started" className="btn btn-large btn-ghost">
            Read the guide
          </Link>
        </div>
      </section>
    </main>
  );
}
