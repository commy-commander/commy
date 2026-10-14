# Vendored design skills

These skills were copied from open source projects so every Claude Code session on this repo can
use them for UI and website work. Each folder keeps its upstream license.

| Skill | Upstream | Commit | License | What was changed |
|---|---|---|---|---|
| `design-taste-frontend` | github.com/Leonxlnx/taste-skill `skills/taste-skill` | f624f37 | MIT | One gallery site entry removed from `library/*.json`. |
| `redesign-existing-projects` | github.com/Leonxlnx/taste-skill `skills/redesign-skill` | f624f37 | MIT | None. |
| `impeccable` | github.com/pbakaus/impeccable `.claude/skills/impeccable` | 2f98774 | Apache-2.0 | `scripts/` (launcher that downloads an engine binary) not vendored; SKILL.md setup rewritten to read project context directly; `live`, `generate`, hooks and doctor commands and their references removed. |
| `img2threejs` | github.com/img2threejs/img2threejs | d508b59 | Apache-2.0 | Only SKILL.md, grimoire/, docs/, forge/ (without tests) and one capture script vendored; checkout section in SKILL.md replaced with a location note. |
| `design-md-examples` | github.com/VoltAgent/awesome-design-md | 3b60532 | MIT | Six DESIGN.md files only; SKILL.md written for this repo. |

To update one, re-copy from upstream at a new commit, re-apply the changes listed above, and update
this table.
