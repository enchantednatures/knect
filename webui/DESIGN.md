# knect WebUI design system

The WebUI is an operational observability surface, not a marketing page. Every
surface exists to answer one of three questions in under a second: *what is
running*, *what is broken*, and *who changed it*. Density, contrast, and stable
spatial memory beat decoration.

## 0. Research log

Greenfield surface, built with embedded references rather than mood words.

| Lane | Deliverable | Decision |
|---|---|---|
| Layer A style | `soft-skill.md` / `taste-skill.md` — calm, high-contrast product surfaces | Adopted the calm operational posture, rejected glassmorphism; data must stay legible at 100% zoom. |
| Layer B reference | `linear.app` + `supabase` design systems | Stole the graphite canvas with one restrained cyan accent, hairline borders, and dense mono data chips. Recombined, never copied. |
| Spatial reference | `stylegallery.md` app-shell pattern | The canvas owns scroll/zoom; the shell (top/cornerbars/drawers) never scrolls with the graph. |
| Domain data | `ui-ux-db` dashboard + graph guidance | Category colour per resource class, health always carries a text label, tables use tabular numerals. |
| Product reference | Lazyweb-style observability consoles (Grafana / Argo CD / k9s) | Learned the inspector-drawer + status-bar grammar; avoided their cramped default type sizes. |

Accepted debt: no bundled webfont. The font stack degrades to platform UI
fonts; the mono stack degrades to `ui-monospace`. This keeps the gzip budget
under the 2 MiB target and avoids a runtime CDN dependency.

## 1. Colour tokens

All values are CSS custom properties in `src/app.css` and exposed to Tailwind
through `@theme`. Components must use tokens, never raw hex.

| Token | Value | Use |
|---|---|---|
| `--bg` | `#090c11` | Page canvas |
| `--surface` | `#111620` | Panels, drawers |
| `--surface-raised` | `#171e2a` | Cards, selected rows |
| `--surface-sunken` | `#0c1017` | Code blocks, inputs |
| `--border` | `#26303f` | Hairlines |
| `--border-strong` | `#38465a` | Focused containers |
| `--text` | `#e8eef7` | Primary text |
| `--text-muted` | `#93a1b5` | Secondary text |
| `--text-faint` | `#66748a` | Metadata |
| `--accent` | `#38bdf8` | Primary action / live connection |
| `--accent-ink` | `#041018` | Text on accent |
| `--ok` | `#34d399` | Healthy |
| `--warn` | `#fbbf24` | Degraded / progressing |
| `--danger` | `#f87171` | Blocked / error |
| `--info` | `#60a5fa` | Informational |

Category colours (node rail + icon only, never the sole signal):

| Category group | Token | Value |
|---|---|---|
| KService | `--cat-kservice` | `#38bdf8` |
| Revision | `--cat-revision` | `#2dd4bf` |
| Broker | `--cat-broker` | `#f59e0b` |
| Trigger | `--cat-trigger` | `#a78bfa` |
| BackingService | `--cat-backing` | `#f472b6` |
| Storage | `--cat-storage` | `#94a3b8` |
| Policy | `--cat-policy` | `#fb7185` |
| Everything else | `--cat-generic` | `#7c8da6` |

## 2. Typography

- UI/display: `"Avenir Next", "Segoe UI Variable", "IBM Plex Sans", system-ui, sans-serif`
- Data/mono: `"JetBrains Mono", "SFMono-Regular", ui-monospace, monospace`
- Scale: 11px micro, 12px meta, 13px body, 15px panel title, 20px app title, 28px empty-state title.
- Data uses `font-variant-numeric: tabular-nums`.

## 3. Spacing, radii, elevation

- Spacing scale: 4, 8, 12, 16, 20, 24, 32, 40, 48 px.
- Radii: 4px chips, 8px controls, 12px cards/nodes, 16px drawers.
- Elevation: one shadow token `--shadow-panel: 0 18px 48px -24px rgb(0 0 0 / .9)`.
  No stacked drop shadows.

## 4. Primitives

`NodeShell` is the single node primitive: category rail, icon, title,
namespace, health label, summary, badges, handles. Every custom node composes
it; no node re-implements layout.

Panel primitives: `PanelSection`, `Chip`, `KeyValue`, `EmptyState`, `StatPill`.
Drawers use the same surface/border tokens as the canvas overlays.

## 5. Motion

- `--motion-fast: 120ms`, `--motion-base: 200ms`, ease `cubic-bezier(.2,.8,.2,1)`.
- Motion only marks state: drawer slide, selection ring, edge flow for live
  traffic and tap events. Nothing decorative.
- `prefers-reduced-motion: reduce` stops edge flow and collapses transitions to
  0.01ms. The UI remains fully usable.

## 6. Accessibility constraints

- Every health/connection state carries a text label; colour is redundant.
- Focus-visible ring: 2px `--accent` with 2px offset, on dark and raised surfaces.
- Graph nodes are focusable buttons with an `aria-label` naming kind, name,
  namespace, and health. Drawers trap nothing; Escape closes them and returns
  focus to the originating node.
- Minimum touch target 40x40 CSS px on the 390px breakpoint.
- Contrast: body text ≥ 7:1 on `--bg`, muted text ≥ 4.5:1.

## 7. Responsive contract

- ≥1280px: top bar, left filter rail, right inspector drawer, bottom status bar.
- 768–1279px: filter rail collapses into a top-left toggle; inspector overlays.
- ≤767px (target 390x844): one canvas, top bar wraps to two rows, drawers are
  full-width sheets, minimap hidden, controls stay reachable.

## 8. Accepted debt

- No virtualisation of the tap event list; the daemon caps events per tap.
- Graph layout is full-dagre recomputation on structural change only. Node
  updates keep previous positions, so SSE ticks never reflow the canvas.
- No light theme. The surface is a dark operational console.
