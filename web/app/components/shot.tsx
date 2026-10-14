// A screenshot in an app window frame. Captures are 1440x960 at 1.5x.
export default function Shot({ src, alt, priority = false }: { src: string; alt: string; priority?: boolean }) {
  return (
    <figure className="shot">
      <div className="shot-bar" aria-hidden="true">
        <span />
        <span />
        <span />
      </div>
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src={src}
        alt={alt}
        width={2160}
        height={1440}
        loading={priority ? "eager" : "lazy"}
        decoding="async"
        {...(priority ? ({ fetchpriority: "high" } as Record<string, string>) : {})}
      />
    </figure>
  );
}
