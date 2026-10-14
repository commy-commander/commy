"use client";

import { useEffect, useRef, useState } from "react";

export type Shot = { id: string; src: string; alt: string };

// The pinned half of the split stage. It shows one app screenshot at a time and
// changes to the one named by whichever [data-shot] chapter is nearest the middle
// of the viewport. Without script, or on a phone, the first shot simply stays.
export default function Stage({ shots, root }: { shots: Shot[]; root: string }) {
  const [active, setActive] = useState(shots[0]?.id ?? "");
  const activeRef = useRef(active);

  useEffect(() => {
    const scope = document.getElementById(root);
    if (!scope || !("IntersectionObserver" in window)) return;
    const chapters = Array.from(scope.querySelectorAll<HTMLElement>("[data-shot]"));
    if (chapters.length === 0) return;

    const pick = () => {
      const middle = window.innerHeight * 0.5;
      let best: HTMLElement | null = null;
      let bestDistance = Infinity;
      for (const el of chapters) {
        const r = el.getBoundingClientRect();
        // The chapter whose block contains, or sits nearest, the middle of the screen wins.
        const distance = r.top > middle ? r.top - middle : r.bottom < middle ? middle - r.bottom : 0;
        if (distance < bestDistance) {
          bestDistance = distance;
          best = el;
        }
      }
      const next = best?.dataset.shot ?? shots[0]?.id ?? "";
      if (next !== activeRef.current) {
        activeRef.current = next;
        setActive(next);
      }
    };

    const io = new IntersectionObserver(pick, { threshold: [0, 0.25, 0.5, 0.75, 1] });
    for (const el of chapters) io.observe(el);
    pick();
    let raf = 0;
    const onScroll = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(pick);
    };
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      io.disconnect();
      window.removeEventListener("scroll", onScroll);
      cancelAnimationFrame(raf);
    };
  }, [root, shots]);

  return (
    <div className="stage-frame" aria-live="off">
      {shots.map((s, i) => (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          key={s.id}
          src={s.src}
          alt={s.alt}
          width={2160}
          height={1440}
          loading={i === 0 ? "eager" : "lazy"}
          decoding="async"
          data-active={s.id === active ? "" : undefined}
          {...(i === 0 ? ({ fetchpriority: "high" } as Record<string, string>) : {})}
        />
      ))}
    </div>
  );
}
