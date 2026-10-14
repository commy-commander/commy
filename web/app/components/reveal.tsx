"use client";

import { usePathname } from "next/navigation";
import { useEffect } from "react";

// Fades [data-reveal] elements in as they scroll into view. Content stays
// visible without JavaScript and for visitors who prefer reduced motion.
export default function Reveal() {
  const pathname = usePathname();

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches || !("IntersectionObserver" in window)) return;
    const root = document.documentElement;
    const items = Array.from(document.querySelectorAll<HTMLElement>("[data-reveal]:not(.is-visible)"));
    // Anything already on screen shows at once so the first paint never flashes empty.
    for (const el of items) {
      if (el.getBoundingClientRect().top < window.innerHeight * 0.9) el.classList.add("is-visible");
    }
    root.classList.add("reveal-ready");
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          entry.target.classList.add("is-visible");
          io.unobserve(entry.target);
        }
      },
      { rootMargin: "0px 0px -10% 0px", threshold: 0.1 },
    );
    for (const el of items) if (!el.classList.contains("is-visible")) io.observe(el);
    return () => io.disconnect();
  }, [pathname]);

  return null;
}
