// The starfield behind every page: drawn once on the server with a fixed seed,
// so the same sky renders for everyone and nothing is loaded for it. A few stars
// twinkle slowly; the rest hold still. Reduced motion stops the twinkle.

function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

const W = 1600;
const H = 1000;
const COUNT = 230;

type Star = { x: number; y: number; r: number; o: number; twinkle: boolean; d: number };

function stars(): Star[] {
  const r = rng(20261014);
  const out: Star[] = [];
  for (let i = 0; i < COUNT; i++) {
    const size = r();
    out.push({
      x: Math.round(r() * W * 10) / 10,
      y: Math.round(r() * H * 10) / 10,
      r: size < 0.82 ? 0.9 : size < 0.96 ? 1.5 : 2.3,
      o: Math.round((0.35 + r() * 0.6) * 100) / 100,
      twinkle: i % 4 === 0,
      d: Math.round(r() * 60) / 10,
    });
  }
  return out;
}

const STARS = stars();

export default function Sky() {
  return (
    <svg className="sky" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="xMidYMid slice" aria-hidden="true" focusable="false">
      {STARS.map((s, i) => (
        <circle
          key={i}
          cx={s.x}
          cy={s.y}
          r={s.r}
          fill="currentColor"
          opacity={s.o}
          data-twinkle={s.twinkle ? "" : undefined}
          style={s.twinkle ? ({ "--d": s.d } as React.CSSProperties) : undefined}
        />
      ))}
    </svg>
  );
}
