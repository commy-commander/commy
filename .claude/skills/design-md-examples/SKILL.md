---
name: design-md-examples
description: Example DESIGN.md files (plain-text design systems with tokens, type, components and rules) for writing or updating Commy's own DESIGN.md, or for studying how a dark, dense product UI is specified. Use when creating a DESIGN.md, defining design tokens, or when a design skill asks for a DESIGN.md.
license: MIT
---

# DESIGN.md examples

A DESIGN.md is a markdown design system an agent reads before building UI: a YAML front block of
tokens (colors, typography, spacing, radii, shadows) followed by prose rules for surfaces,
components, motion and do/don't lists.

`examples/` holds six analyses of public product websites, chosen because they are dark, dense
software UIs close to what Commy needs:

| File | Useful for |
|---|---|
| `examples/linear.app.md` | near-black surface ladder, single accent, hairline borders |
| `examples/vercel.md` | monochrome type-led system, light and dark |
| `examples/warp.md` | terminal-flavoured dark app chrome |
| `examples/supabase.md` | dark developer dashboard with one brand green |
| `examples/sentry.md` | dense data UI, status colors |
| `examples/notion.md` | calm light UI, reading surfaces |

## Rules for using them

- They are format and craft references, not themes. Never copy another product's palette, logo,
  type, copy or layout into Commy; Commy's own DESIGN.md and existing UI win.
- To write Commy's DESIGN.md, extract tokens from the real code first (`apps/desktop/ui/` CSS for
  the app, `web/` for the website), then follow the structure of these files.
- Brand names in these files are the analysed sites; never repeat them in Commy's UI, docs or copy.

Source: VoltAgent/awesome-design-md (MIT, see `LICENSE`). Only six of the collection's files are
vendored.
