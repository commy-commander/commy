import { FACE } from "./mascot";

// Commy as a cosmonaut: the app's own face inside a round helmet, a cream suit
// with the red star patch, one arm up in a wave. Flat shapes only, so it sits on
// the starfield like a sticker and recolours with the page.
export default function Cosmonaut({ className, title }: { className?: string; title?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 600 760"
      role={title ? "img" : undefined}
      aria-hidden={title ? undefined : true}
      focusable="false"
    >
      {title && <title>{title}</title>}
      {/* backpack */}
      <rect x="150" y="430" width="300" height="230" rx="60" fill="#c9c6bd" />
      {/* raised arm, waving */}
      <g transform="rotate(42 440 470)">
        <rect x="402" y="300" width="76" height="200" rx="38" fill="#f2efe6" />
        <circle cx="440" cy="300" r="46" fill="#ff4d4d" />
        <rect x="402" y="330" width="76" height="22" fill="#c9c6bd" />
      </g>
      {/* torso */}
      <rect x="168" y="420" width="264" height="250" rx="70" fill="#f2efe6" />
      <rect x="168" y="548" width="264" height="36" fill="#ff4d4d" />
      {/* other arm */}
      <rect x="104" y="470" width="76" height="190" rx="38" fill="#f2efe6" />
      <circle cx="142" cy="660" r="44" fill="#ff4d4d" />
      <rect x="104" y="610" width="76" height="22" fill="#c9c6bd" />
      {/* boots */}
      <rect x="196" y="640" width="90" height="100" rx="34" fill="#f2efe6" />
      <rect x="314" y="640" width="90" height="100" rx="34" fill="#f2efe6" />
      <rect x="190" y="700" width="102" height="50" rx="22" fill="#ff4d4d" />
      <rect x="308" y="700" width="102" height="50" rx="22" fill="#ff4d4d" />
      {/* chest patch with the star */}
      <rect x="252" y="462" width="96" height="70" rx="16" fill="#ff4d4d" />
      <path
        transform="translate(300 497) scale(1.25)"
        d="M0 -16 L4.7 -5.2 L16.5 -4.9 L7.2 2.5 L10.2 13.9 L0 7.4 L-10.2 13.9 L-7.2 2.5 L-16.5 -4.9 L-4.7 -5.2 Z"
        fill="#ffffff"
      />
      {/* neck ring */}
      <rect x="206" y="404" width="188" height="44" rx="22" fill="#c9c6bd" />
      {/* helmet: dark glass, cream rim, a highlight */}
      <circle cx="300" cy="250" r="214" fill="#151929" />
      <circle cx="300" cy="250" r="214" fill="none" stroke="#f2efe6" strokeWidth="16" />
      <g transform="translate(74 -18) scale(0.135)" dangerouslySetInnerHTML={{ __html: FACE }} />
      <path d="M150 170 Q190 90 280 70" fill="none" stroke="#ffffff" strokeWidth="14" strokeLinecap="round" opacity="0.55" />
      <circle cx="118" cy="222" r="9" fill="#ffffff" opacity="0.55" />
    </svg>
  );
}
