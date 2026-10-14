import Link from "next/link";
import { headers } from "next/headers";
import { getLatestRelease } from "@/lib/releases";
import { detectPlatform, PLATFORM_NAMES } from "@/lib/release-assets";
import { REPO_URL } from "@/lib/site";
import InviteDemo from "./components/invite-demo";
import Mascot from "./components/mascot";
import Stage, { type Shot } from "./components/stage";

const SHOTS: Shot[] = [
  { id: "chat", src: "/screenshots/chat.jpg", alt: "A Commy server, Pixel Pioneers, with a busy #general channel: an image, a shared file, reactions, a reply and an invite card" },
  { id: "call", src: "/screenshots/call.jpg", alt: "Commy's voice connection panel showing ping, packet loss, jitter and a direct route for each person in the call" },
  { id: "voice", src: "/screenshots/voice.jpg", alt: "Commy's Voice and Audio settings with the Noise Cancellation switch on and a mic test" },
  { id: "stream", src: "/screenshots/stream.jpg", alt: "Watching a member's live racing game stream inside a Commy voice channel, 1280 by 720 at 50 frames a second" },
  { id: "roles", src: "/screenshots/roles.jpg", alt: "Commy's server settings on the Roles page: Admin, Moderator, Racer and Builder roles with their colours and permissions" },
  { id: "signin", src: "/screenshots/signin.jpg", alt: "Commy's sign-in dialog showing a one-time code to approve in the browser and the account server choice" },
];

type Chapter = { shot: string; title: string; body: string; lines: string[] };

const CHAPTERS: Chapter[] = [
  {
    shot: "chat",
    title: "Channels that feel lived in",
    body: "Text channels for every topic, with replies and threads to keep busy nights readable, reactions and custom emoji, and images, GIFs and files dropped straight into the chat.",
    lines: ["Replies, threads and mentions", "Custom emoji for every server", "Files up to 512 MB, sent member to member"],
  },
  {
    shot: "call",
    title: "Click in. Start talking.",
    body: "Join a voice channel and you are in. Audio goes straight between members over an encrypted connection and is encoded with Opus, so it stays clear on any network.",
    lines: ["Direct routes between people, a relay only as fallback", "Live ping, packet loss and route for every person", "Per-person volume, mute and deafen in one click"],
  },
  {
    shot: "voice",
    title: "One switch and the room goes quiet",
    body: "Keyboards, fans and the dishwasher stay out of the call. Noise cancellation is a single toggle for everyone, with a mic test that plays you back through the whole pipeline.",
    lines: ["On or off from settings or the voice panel", "Hear yourself before anyone else does", "Input, output and camera devices in one place"],
  },
  {
    shot: "stream",
    title: "Go live with your game in two clicks",
    body: "Share a screen or a single window with everyone in the channel. Pick Smooth for games or Sharp for code and documents, and watch together in focus or fullscreen.",
    lines: ["Smooth, Balanced and Sharp presets", "Resolution and frame rate shown live", "Webcams and a game overlay come along"],
  },
  {
    shot: "roles",
    title: "Run the server your way",
    body: "Roles with colours, tags and fine permissions. Channels and categories you drag into place, rules members accept on entry, events with RSVPs, and moderation tools for when things get loud.",
    lines: ["Roles members can pick for themselves", "Mute, kick, ban and member reports", "Rules gate, AFK channel, soundboard"],
  },
  {
    shot: "signin",
    title: "No one listening in",
    body: "Messages, voice, video, streams and files travel directly between devices. A free account only proves you are a real person, which keeps spam bots out, and you can run your own account server if you would rather not trust ours.",
    lines: ["No company server in your chats or calls", "No ads, no analytics, no selling data", "Host the whole stack yourself"],
  },
];

const INCLUDED: { title: string; shot: string; pos: string; zoom: number; lines: string[] }[] = [
  {
    title: "Share anything",
    shot: "/screenshots/chat.jpg",
    pos: "30% 28%",
    zoom: 2.2,
    lines: ["Images and GIFs inline", "Files up to 512 MB", "Custom emoji and reactions", "Soundboards into voice", "Friends and direct messages"],
  },
  {
    title: "Voice that tells you the truth",
    shot: "/screenshots/call.jpg",
    pos: "12% 66%",
    zoom: 2.6,
    lines: ["Ping, loss and jitter per person", "Direct or relayed, shown live", "Per-person volume", "Webcams in any voice channel", "Opus at 48 kHz"],
  },
  {
    title: "Stream the way you want",
    shot: "/screenshots/stream.jpg",
    pos: "48% 30%",
    zoom: 1.6,
    lines: ["Whole screen or one window", "Smooth, Balanced or Sharp", "Focus and fullscreen viewing", "Game overlay while you play", "Resolution and fps for viewers"],
  },
  {
    title: "Mod tools that stay out of the way",
    shot: "/screenshots/roles.jpg",
    pos: "46% 44%",
    zoom: 2.4,
    lines: ["Roles, colours, tags, permissions", "Pickable roles for members", "Server rules and an AFK channel", "Events with RSVPs", "Mute, kick, ban, reports"],
  },
];

export default async function HomePage() {
  const platform = detectPlatform(headers().get("user-agent"));
  const release = await getLatestRelease();
  const direct = release?.downloads.find((d) => d.platform === platform && d.primary);

  const download = (large: boolean) =>
    direct ? (
      <a href={direct.url} className={`btn btn-primary ${large ? "btn-large" : ""}`}>
        Download for {PLATFORM_NAMES[direct.platform]}
      </a>
    ) : (
      <Link href="/download" className={`btn btn-primary ${large ? "btn-large" : ""}`}>
        Download Commy
      </Link>
    );

  return (
    <main className="home">
      <section className="stage" id="stage" aria-label="Commy, the app">
        <div className="stage-left field-night">
          <Stage shots={SHOTS} root="stage" />
        </div>
        <div className="stage-right">
          <div className="hero">
            <h1 data-rise style={{ "--i": 0 } as React.CSSProperties}>
              Your people.
              <br />
              No middleman.
            </h1>
            <p className="lead" data-rise style={{ "--i": 1 } as React.CSSProperties}>
              Commy is free, open source voice and text chat for your community. It runs between your members&apos;
              computers, so there is nothing to rent, no paywall, and no company in the middle.
            </p>
            <div className="hero-actions" data-rise style={{ "--i": 2 } as React.CSSProperties}>
              {download(true)}
            </div>
          </div>

          <p className="wordmark-giant" aria-hidden="true">
            commy
          </p>

          <div className="chapters">
            {CHAPTERS.map((c, i) => (
              <article key={c.shot} className="chapter" data-shot={c.shot} id={c.shot}>
                <h2 data-reveal style={{ "--i": 0 } as React.CSSProperties}>
                  {c.title}
                </h2>
                <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
                  {c.body}
                </p>
                <ul data-reveal style={{ "--i": 2 } as React.CSSProperties}>
                  {c.lines.map((l) => (
                    <li key={l}>{l}</li>
                  ))}
                </ul>
                <figure className="chapter-shot field-night" data-reveal style={{ "--i": 3 } as React.CSSProperties}>
                  {/* eslint-disable-next-line @next/next/no-img-element */}
                  <img src={SHOTS[i].src} alt={SHOTS[i].alt} width={2160} height={1440} loading="lazy" decoding="async" />
                </figure>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className="invite" aria-labelledby="invite-title">
        <InviteDemo />
        <div className="invite-field field-night">
          <h2 id="invite-title" data-reveal style={{ "--i": 0 } as React.CSSProperties}>
            Send a link.
          </h2>
          <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
            Every server gets short commy.chat links. Try one: name a server and read the link the app would hand you.
          </p>
        </div>
        <div className="invite-field invite-field-light">
          <h2 aria-hidden="true" data-reveal style={{ "--i": 2 } as React.CSSProperties}>
            They&apos;re in.
          </h2>
          <p data-reveal style={{ "--i": 3 } as React.CSSProperties}>
            Your friend clicks, Commy opens, and this is all they see before they are in the channel with you.
          </p>
        </div>
      </section>

      <section className="included field-accent" aria-labelledby="included-title">
        <h2 id="included-title">
          All of it.
          <br />
          Free.
        </h2>
        <div className="included-blocks">
          {INCLUDED.map((b, i) => (
            <div className={`included-block ${i % 2 ? "flip" : ""}`} key={b.title}>
              <h3 data-reveal style={{ "--i": 0 } as React.CSSProperties}>
                {b.title}
              </h3>
              <div className="included-row">
                <div className="crop" data-reveal style={{ "--i": 1, "--pos": b.pos, "--zoom": b.zoom } as React.CSSProperties}>
                  {/* eslint-disable-next-line @next/next/no-img-element */}
                  <img src={b.shot} alt="" width={2160} height={1440} loading="lazy" decoding="async" />
                </div>
                <ul data-reveal style={{ "--i": 2 } as React.CSSProperties}>
                  {b.lines.map((l) => (
                    <li key={l}>{l}</li>
                  ))}
                </ul>
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="howto" aria-labelledby="howto-title">
        <h2 id="howto-title" data-reveal>
          Up and running in a minute
        </h2>
        <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
          <strong>To begin,</strong> download Commy for Windows, macOS or Linux and sign in with a free account. The
          account is only there to prove you are a person; it never sees a message or a call.
        </p>
        <div className="evidence field-night" data-reveal style={{ "--i": 2 } as React.CSSProperties}>
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/screenshots/signin.jpg" alt="The sign-in code dialog in the app" width={2160} height={1440} loading="lazy" decoding="async" style={{ "--pos": "50% 46%", "--zoom": 1.9 } as React.CSSProperties} />
        </div>
        <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
          <strong>Within a minute,</strong> create a server. Add channels, roles, rules, emoji and a soundboard. It
          lives on your members&apos; computers, not on ours, and it is yours for as long as you want it.
        </p>
        <div className="evidence field-night" data-reveal style={{ "--i": 2 } as React.CSSProperties}>
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/screenshots/chat.jpg" alt="The channel list of a new server" width={2160} height={1440} loading="lazy" decoding="async" style={{ "--pos": "4% 22%", "--zoom": 2.4 } as React.CSSProperties} />
        </div>
        <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
          <strong>After that,</strong> share an invite link. Friends click it, Commy opens, and they join you directly.
          The link expires on its own and you can revoke it whenever you like.
        </p>
        <div className="evidence field-night" data-reveal style={{ "--i": 2 } as React.CSSProperties}>
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/screenshots/invite.jpg" alt="The join prompt a friend sees after clicking an invite link" width={2160} height={1440} loading="lazy" decoding="async" style={{ "--pos": "50% 50%", "--zoom": 2.4 } as React.CSSProperties} />
        </div>
        <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
          <strong>From then on,</strong> it is just your people. No premium tier, no boosts to buy, and the{" "}
          <Link href="/docs/getting-started">guide</Link> for anything you want to set up next.
        </p>
      </section>

      <section className="letter" aria-label="A note from the maker">
        <Mascot className="letter-mark" />
        <p data-reveal style={{ "--i": 0 } as React.CSSProperties}>
          I built Commy because the places my friends and I hang out online kept getting worse: more paywalls, more ads,
          more of our conversations sitting on someone else&apos;s servers.
        </p>
        <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
          So Commy connects people directly. Your messages, calls, streams and files go from your computer to your
          friends&apos; computers, encrypted, with a relay only when a direct route cannot be found. There is no
          company in the middle because there is nothing in the middle.
        </p>
        <p data-reveal style={{ "--i": 2 } as React.CSSProperties}>
          That is also why it is free. Nobody is renting a server to run your community, so there is nothing to charge
          for. Every feature is there for every member of every server, and the whole thing is{" "}
          <a href={REPO_URL} target="_blank" rel="noreferrer">
            open source
          </a>{" "}
          under the AGPL, so it stays that way.
        </p>
        <p className="letter-sign" data-reveal style={{ "--i": 3 } as React.CSSProperties}>
          Commy Commander
          <span>maker of Commy</span>
        </p>
      </section>

      <section className="close field-night" aria-labelledby="close-title">
        <div className="close-copy">
          <h2 id="close-title">Bring your people.</h2>
          <p>Free forever, open source, and ready for Windows, macOS and Linux.</p>
          <div className="hero-actions">
            {download(true)}
            <Link href="/docs/getting-started" className="btn btn-ghost btn-large">
              Read the guide
            </Link>
          </div>
        </div>
        <p className="wordmark-close" aria-hidden="true">
          commy
        </p>
      </section>
    </main>
  );
}
