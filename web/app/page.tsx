import Link from "next/link";
import { headers } from "next/headers";
import { getLatestRelease } from "@/lib/releases";
import { detectPlatform, PLATFORM_NAMES } from "@/lib/release-assets";
import { REPO_URL } from "@/lib/site";
import Cosmonaut from "./components/cosmonaut";
import InviteDemo from "./components/invite-demo";

type Feature = {
  id: string;
  title: React.ReactNode;
  body: string;
  lines: string[];
  shot: string;
  alt: string;
  pos: string;
  zoom: number;
  fragment: React.ReactNode;
};

// Small true pieces of the interface, drawn the way the app draws them, so each
// feature shows the control itself rather than an icon standing in for it.
function ChatFragment() {
  return (
    <div className="ui ui-message" data-ui role="img" aria-label="A chat message with a GIF and reactions, as shown in Commy">
      <span className="ui-avatar ui-avatar-nova" aria-hidden="true" />
      <div>
        <p className="ui-author">
          nova <span>Today at 4:14 PM</span>
        </p>
        <p className="ui-source">static.klipy.com</p>
        <span className="ui-gif" />
        <p className="ui-reactions">
          <span className="is-mine">😻 5</span>
          <span>👋 3</span>
          <span className="ui-add">+</span>
        </p>
      </div>
    </div>
  );
}

function VoiceFragment() {
  const people = [
    { name: "ferris", quality: "Good", ping: "23 ms", route: "Direct", tone: "a" },
    { name: "nova", quality: "Good", ping: "31 ms", route: "Direct", tone: "b" },
    { name: "sprocket", quality: "Fair", ping: "140 ms", route: "Via relay", tone: "c" },
  ];
  return (
    <div className="ui ui-conn" data-ui role="img" aria-label="The voice connection panel: ping and route for each person">
      <p className="ui-head">
        Voice Connection <span>Up 800 kbps · Down 747 kbps</span>
      </p>
      {people.map((p) => (
        <div className="ui-row" key={p.name}>
          <span className={`ui-avatar ui-avatar-${p.tone}`} aria-hidden="true" />
          <span className="ui-name">{p.name}</span>
          <span className="ui-value">{p.ping}</span>
          <span className="ui-value">{p.route}</span>
          <span className={`ui-pill ${p.quality === "Good" ? "is-good" : "is-fair"}`}>{p.quality}</span>
        </div>
      ))}
    </div>
  );
}

function StreamFragment() {
  return (
    <div className="ui ui-stream" data-ui role="img" aria-label="Screen share controls: a live badge, the resolution and frame rate, and the Smooth, Balanced and Sharp presets">
      <p className="ui-head">
        <span className="ui-live">LIVE</span> sprocket&apos;s screen <span>1280×720 · 50 fps</span>
      </p>
      <div className="ui-presets">
        <span className="is-on">Smooth</span>
        <span>Balanced</span>
        <span>Sharp</span>
      </div>
      <p className="ui-note">Smooth for games, Sharp for code and documents.</p>
    </div>
  );
}

function PrivateFragment() {
  return (
    <div className="ui ui-private" data-ui role="img" aria-label="What the account server sees, next to what it never sees">
      <div>
        <p className="ui-head">commy.chat sees</p>
        <ul>
          <li>Your email and display name</li>
          <li>Which devices are yours</li>
          <li>Invite links you create</li>
        </ul>
      </div>
      <div>
        <p className="ui-head">commy.chat never sees</p>
        <ul>
          <li>Messages, images and files</li>
          <li>Voice, video and streams</li>
          <li>Which servers you are in</li>
        </ul>
      </div>
    </div>
  );
}

const FEATURES: Feature[] = [
  {
    id: "chat",
    title: (
      <>
        Chat that feels
        <br />
        lived in
      </>
    ),
    body: "Text channels for every topic, replies and threads for busy nights, custom emoji, reactions, and GIFs, images and files dropped straight into the conversation.",
    lines: ["Built-in GIF search", "Files up to 512 MB, member to member", "Replies, threads, mentions and pins"],
    shot: "/screenshots/chat.jpg",
    alt: "A Commy server, Pixel Pioneers, with a busy #general channel: a shared pixel-art build, a cat GIF, reactions, a reply, a file card and an invite card",
    pos: "42% 72%",
    zoom: 1.5,
    fragment: <ChatFragment />,
  },
  {
    id: "voice",
    title: (
      <>
        Voice that goes
        <br />
        straight to them
      </>
    ),
    body: "Click a voice channel and you are in. Audio travels directly between members, encrypted, with a relay only when no direct route exists. Noise cancellation is one switch for everyone.",
    lines: ["Ping, loss and route shown for every person", "Keyboards and fans stay out of the call", "Per-person volume, mute and deafen"],
    shot: "/screenshots/call.jpg",
    alt: "Commy's voice connection panel showing ping, packet loss, jitter and a direct route for each person in the call",
    pos: "14% 62%",
    zoom: 1.8,
    fragment: <VoiceFragment />,
  },
  {
    id: "stream",
    title: (
      <>
        Go live with your
        <br />
        game in two clicks
      </>
    ),
    body: "Share your screen or a single window with the channel. Pick Smooth for games or Sharp for code, and everyone watches together in focus or fullscreen.",
    lines: ["Resolution and frame rate shown live", "Webcams in any voice channel", "A game overlay while you play"],
    shot: "/screenshots/stream.jpg",
    alt: "Watching a member's live pixel-art farming game inside a Commy voice channel, 1280 by 720 at 50 frames a second",
    pos: "52% 34%",
    zoom: 1.45,
    fragment: <StreamFragment />,
  },
  {
    id: "private",
    title: (
      <>
        No one in
        <br />
        the middle
      </>
    ),
    body: "A free account only proves you are a person, which keeps spam bots out. Everything else goes from your computer to your friends' computers. If you would rather not trust ours, run your own account server.",
    lines: ["Roles, rules, events and mod tools, all yours", "No ads, no analytics, nothing sold", "Host the whole stack yourself"],
    shot: "/screenshots/roles.jpg",
    alt: "Commy's server settings on the Roles page: Admin, Moderator, Racer and Builder roles with their colours and permissions",
    pos: "50% 40%",
    zoom: 1.5,
    fragment: <PrivateFragment />,
  },
];

const INCLUDED = [
  "Text channels, threads and replies",
  "Voice channels with noise cancellation",
  "Screen and game streaming",
  "Webcams and a game overlay",
  "GIF search and custom emoji",
  "Files up to 512 MB",
  "Roles, tags and permissions",
  "Invite links that open the app",
  "Server rules, events and RSVPs",
  "Soundboard and direct messages",
  "Mute, kick, ban and reports",
  "Your own account server, if you like",
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
      <section className="hero" aria-labelledby="hero-title">
        <div className="hero-copy">
          <h1 id="hero-title" data-rise style={{ "--i": 0 } as React.CSSProperties}>
            Your people.
            <br />
            Nothing in between.
          </h1>
          <p className="lead" data-rise style={{ "--i": 1 } as React.CSSProperties}>
            Commy is free, open source voice and text chat for your community. It runs between your members&apos; own
            computers, so there is nothing to rent, no plan to buy, and no company in the middle of your conversations.
          </p>
          <div className="hero-actions" data-rise style={{ "--i": 2 } as React.CSSProperties}>
            {download(true)}
            <a href="#free" className="btn btn-ghost btn-large">
              How it&apos;s free
            </a>
          </div>
        </div>
        <div className="hero-art" data-rise style={{ "--i": 1 } as React.CSSProperties}>
          <Cosmonaut className="cosmonaut" />
        </div>
      </section>

      <figure className="hero-shot" data-rise style={{ "--i": 3 } as React.CSSProperties}>
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src="/screenshots/chat.jpg"
          alt={FEATURES[0].alt}
          width={2160}
          height={1440}
          decoding="async"
          {...({ fetchpriority: "high" } as Record<string, string>)}
        />
      </figure>

      <section className="features" aria-label="What Commy does">
        {FEATURES.map((f, i) => (
          <article className={`feature ${i % 2 ? "flip" : ""}`} key={f.id} id={f.id}>
            <div className="feature-copy">
              <h2 data-reveal style={{ "--i": 0 } as React.CSSProperties}>
                {f.title}
              </h2>
              <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
                {f.body}
              </p>
              <ul data-reveal style={{ "--i": 2 } as React.CSSProperties}>
                {f.lines.map((l) => (
                  <li key={l}>{l}</li>
                ))}
              </ul>
            </div>
            <div className="feature-art" data-reveal style={{ "--i": 1 } as React.CSSProperties}>
              <div className="crop" style={{ "--pos": f.pos, "--zoom": f.zoom } as React.CSSProperties}>
                {/* eslint-disable-next-line @next/next/no-img-element */}
                <img src={f.shot} alt={f.alt} width={2160} height={1440} loading="lazy" decoding="async" />
              </div>
              <div className="fragment">{f.fragment}</div>
            </div>
          </article>
        ))}
      </section>

      <section id="free" className="free field-accent" aria-labelledby="free-title">
        <div className="free-grid">
          <div className="free-copy">
            <h2 id="free-title" data-reveal style={{ "--i": 0 } as React.CSSProperties}>
              Free, because there is nothing to pay for
            </h2>
            <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
              Other chat apps rent racks of servers to carry your messages and calls, then charge you, or your data, to
              cover them. Commy does not have those servers. Your community lives on its members&apos; computers and
              every message, call, stream and file goes directly between them.
            </p>
            <p data-reveal style={{ "--i": 2 } as React.CSSProperties}>
              The only hosted piece is a small account service that keeps spam bots out, and you can run that yourself.
              Nobody is paying to host your server, so nobody has to charge you for it. Every feature is there for every
              member, and the whole thing is{" "}
              <a href={REPO_URL} target="_blank" rel="noreferrer">
                open source
              </a>{" "}
              under the AGPL, so it stays that way.
            </p>
          </div>
          <Constellation />
        </div>
        <h3 className="free-sub" data-reveal>
          Everything, for everyone
        </h3>
        <ul className="free-list" data-reveal style={{ "--i": 1 } as React.CSSProperties}>
          {INCLUDED.map((l) => (
            <li key={l}>{l}</li>
          ))}
        </ul>
      </section>

      <section className="invite" aria-labelledby="invite-title">
        <div className="invite-copy">
          <h2 id="invite-title" data-reveal style={{ "--i": 0 } as React.CSSProperties}>
            Send a link.
            <br />
            They&apos;re in.
          </h2>
          <p data-reveal style={{ "--i": 1 } as React.CSSProperties}>
            Every server gets short commy.chat links that open the app. Name a server and read the link it would hand
            you, and the prompt your friend sees before they are in the channel with you.
          </p>
        </div>
        <InviteDemo />
      </section>

      <section className="close" aria-labelledby="close-title">
        <Cosmonaut className="cosmonaut cosmonaut-close" />
        <div className="close-copy">
          <h2 id="close-title">Ready to bring your people?</h2>
          <p>Free forever, open source, for Windows, macOS and Linux.</p>
          <div className="hero-actions">
            {download(true)}
            <Link href="/docs/getting-started" className="btn btn-ghost btn-large">
              Read the guide
            </Link>
          </div>
        </div>
      </section>
    </main>
  );
}

// Members as stars, joined directly to each other: the whole hosting diagram.
function Constellation() {
  const nodes = [
    [80, 60],
    [250, 30],
    [420, 90],
    [160, 190],
    [340, 230],
    [60, 320],
    [240, 360],
    [430, 330],
  ];
  const links = [
    [0, 1], [1, 2], [0, 3], [1, 3], [1, 4], [2, 4], [3, 4], [3, 5], [3, 6], [4, 6], [4, 7], [5, 6], [6, 7],
  ];
  return (
    <svg className="constellation" viewBox="0 0 500 400" role="img" aria-label="Eight members of a server, each connected directly to the others, with no server in the middle">
      {links.map(([a, b]) => (
        <line key={`${a}-${b}`} x1={nodes[a][0]} y1={nodes[a][1]} x2={nodes[b][0]} y2={nodes[b][1]} stroke="currentColor" strokeWidth="2" opacity="0.5" />
      ))}
      {nodes.map(([x, y], i) => (
        <g key={i}>
          <circle cx={x} cy={y} r="14" fill="currentColor" />
          <circle cx={x} cy={y} r="22" fill="none" stroke="currentColor" strokeWidth="2" opacity="0.5" />
        </g>
      ))}
      <text x="250" y="395" textAnchor="middle" fill="currentColor" fontSize="14" fontWeight="700" letterSpacing="0.08em">
        YOUR SERVER, HOSTED BY ITS MEMBERS
      </text>
    </svg>
  );
}
