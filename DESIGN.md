# Design System: BearWisdom

## 1. Visual Theme & Atmosphere

Scholar's Workshop — a warm, dark-mode-native environment that evokes an old study room with leather, copper instruments, and teal ink. The palette is built on deep leather-brown backgrounds with copper as the primary accent and teal for interactive/link elements. Sage green marks success states. The warmth is deliberate — this is a tool for deep code exploration, not a sterile IDE panel.

Density is medium-high: this is a developer tool with sidebars, timelines, code viewers, and graph canvases. But density does not mean tiny text — readability at sustained viewing distances is non-negotiable. The smallest text in the system is 12px; labels and metadata use 13px; body content uses 14–15px.

Depth is communicated through background luminance stepping (`#100D09` → `#1A1410` → `#242019` → `#2E2820`) rather than shadows. Shadows exist but are subtle and dark-tinted. Borders use solid warm tones (`#3D3226`) rather than semi-transparent white, maintaining the warm atmosphere.

**Key Characteristics:**
- Dark-mode-native: `#100D09` canvas, `#1A1410` base, `#242019` surface, `#2E2820` elevated
- Warm copper accent (`#C8915C`) — CTAs, active states, data emphasis
- Teal interactive (`#2D8F8F`) — links, focus rings, navigation highlights
- Serif logo (`Playfair Display`), geometric sans body (`DM Sans`), monospace code (`Source Code Pro`)
- Warm solid borders (`#3D3226`), not semi-transparent white
- Minimum text size: 12px. No 9px, 10px, or 11px text anywhere in the UI

## 2. Color Palette & Roles

### Background Surfaces
- **Deepest** (`#100D09`): Page canvas, behind all panels
- **Base** (`#1A1410`): Main content areas, code viewer background
- **Surface** (`#242019`): Panels, sidebars, headers, elevated sections
- **Elevated** (`#2E2820`): Dropdowns, popovers, search results
- **Hover** (`#38302A`): Hover state for interactive rows and items
- **Active** (`#443A30`): Active/pressed state for interactive rows

### Text & Content
- **Primary Text** (`#F0E6D6`): Headings, symbol names, primary content — warm near-white
- **Secondary Text** (`#B8A892`): Body text, descriptions, reference items
- **Muted Text** (`#8B7D6B`): Metadata, timestamps, section headers, placeholders
- **Faint Text** (`#5C5044`): Disabled states, decorative labels, line numbers

### Brand & Accent
- **Copper** (`#C8915C`): Primary accent — active tabs, stat values, data highlights, CTAs
- **Copper Bright** (`#E4AD74`): Hover state for copper elements
- **Copper Dim** (`#8B6B3E`): Muted copper for secondary emphasis
- **Copper Glow** (`#C8915C33`): Subtle background tint, glow shadows

### Interactive / Links
- **Teal** (`#2D8F8F`): Links, file paths, navigation breadcrumbs
- **Teal Bright** (`#3CB8B8`): Hover state for teal elements
- **Teal Dim** (`#1E6B6B`): Borders on teal-tinted badges
- **Teal Glow** (`#2D8F8F33`): Focus rings, subtle teal backgrounds

### Status Colors
- **Success / Sage** (`#62A578`): Success states, positive token savings
- **Warning** (`#D9A85B`): Warnings, number syntax highlighting
- **Error** (`#C87272`): Destructive actions, error states
- **Info** (`#5B8DD9`): Informational badges, search-category highlights

### Border & Divider
- **Border Default** (`#3D3226`): Standard borders between panels, rows, sections
- **Border Subtle** (`#2E2820`): Lightest borders — within panels, between sub-sections

### Graph Node Kind Colors
- **Class / Struct** (`#58A6FF`): Blue
- **Interface** (`#BC8CFF`): Purple
- **Method / Function** (`#3FB950`): Green
- **Enum** (`#D29922`): Gold
- **Module** (`#39C5CF`): Cyan
- **Field** (`#8B949E`): Gray

### Dark Mode Overrides
Dark-mode-native — no light mode variant exists.

## 3. Typography Rules

### Font Family
- **Display / Logo**: `'Playfair Display', Georgia, 'Times New Roman', serif`
- **Primary**: `'DM Sans', -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif`
- **Monospace**: `'Source Code Pro', ui-monospace, 'SF Mono', 'Cascadia Mono', 'Segoe UI Mono', monospace`

### Hierarchy

| Role | Font | Size | Weight | Line Height | Letter Spacing | Use |
|------|------|------|--------|-------------|----------------|-----|
| Display | Playfair Display | 24px (1.5rem) | 700 | 1.2 | -0.01em | Logo, page-level titles |
| Heading 1 | DM Sans | 20px (1.25rem) | 700 | 1.3 | -0.01em | Panel titles, primary headings |
| Heading 2 | DM Sans | 18px (1.125rem) | 600 | 1.35 | -0.005em | Section headings, detail names |
| Heading 3 | DM Sans | 16px (1rem) | 600 | 1.4 | 0 | Sub-section headings |
| Body Large | DM Sans | 15px (0.9375rem) | 400 | 1.6 | 0 | Primary content, search results |
| Body | DM Sans | 14px (0.875rem) | 400 | 1.5 | 0 | Default text, descriptions, references |
| Body Small | DM Sans | 13px (0.8125rem) | 400 | 1.5 | 0 | Compact lists, breadcrumbs, stat labels |
| Caption | DM Sans | 12px (0.75rem) | 500 | 1.4 | 0 | Timestamps, line numbers, minimal metadata |
| Label | DM Sans | 13px (0.8125rem) | 700 | 1.3 | 0.04em | Section labels, uppercase headers (uppercase + tracking) |
| Tab | DM Sans | 13px (0.8125rem) | 700 | 1.3 | 0.04em | Tab buttons (uppercase) |
| Mono Body | Source Code Pro | 14px (0.875rem) | 400 | 1.6 | 0 | Code blocks, file paths, symbol names |
| Mono Small | Source Code Pro | 13px (0.8125rem) | 400 | 1.5 | 0 | Inline code, session IDs, stat values |
| Mono Caption | Source Code Pro | 12px (0.75rem) | 400 | 1.4 | 0 | Line numbers in code viewer |
| Badge | Source Code Pro | 12px (0.75rem) | 700 | 1 | 0 | Kind badges, tool badges |

### Principles
- **Minimum 12px**: Nothing in the UI goes below 12px. Period. The previous 9–11px sizes are banned.
- **Weight system**: 400 (reading), 500 (captions/UI), 600 (sub-headings), 700 (headings, labels, badges)
- **Uppercase + tracking for labels only**: Section headers, tab labels, and overlines use `text-transform: uppercase` with `letter-spacing: 0.04em`. Body text never uses uppercase.
- **Monospace for data**: File paths, symbol names, code, session IDs, numeric values. Never for body prose.

## 4. Component Stylings

### Buttons

**Primary Button (CTA)**
- Background: `var(--copper)` (`#C8915C`)
- Text: `var(--bg-base)` (`#1A1410`)
- Padding: `8px 16px`
- Radius: `var(--radius-sm)` (4px)
- Border: none
- Font: DM Sans, 14px, weight 600
- Hover: `opacity: 0.85`
- Focus: `box-shadow: 0 0 0 3px var(--copper-glow)`
- Active: `opacity: 0.75`
- Disabled: `opacity: 0.4, cursor: not-allowed`
- Use: Primary actions (Embed, Index, Submit)

**Ghost Button**
- Background: `transparent`
- Text: `var(--text-muted)` (`#8B7D6B`)
- Padding: `8px 16px`
- Radius: `var(--radius-sm)` (4px)
- Border: none
- Font: DM Sans, 14px, weight 600
- Hover: `background: var(--bg-hover); color: var(--text-primary)`
- Use: Secondary actions, close buttons, toggles

**Destructive Button**
- Background: `transparent`
- Text: `var(--text-faint)` (`#5C5044`)
- Hover: `color: #C87272; background: rgba(200, 114, 114, 0.1)`
- Use: Delete actions (session delete, etc.)

**Icon Button**
- Size: `32px x 32px`
- Background: `transparent`
- Color: `var(--text-muted)`
- Radius: `var(--radius-sm)` (4px)
- Hover: `background: var(--bg-hover); color: var(--text-primary)`
- Use: Close, collapse, toggle buttons

### Tab Bar

- Container: `background: var(--bg-surface); border-bottom: 1px solid var(--border)`
- Tab: `padding: 8px 16px; font: 13px DM Sans weight 700; uppercase; letter-spacing: 0.04em`
- Inactive: `color: var(--text-faint); border-bottom: 2px solid transparent`
- Hover: `color: var(--text-secondary)`
- Active: `color: var(--copper); border-bottom-color: var(--copper)`

### Cards & Containers
- Background: `var(--bg-surface)` (`#242019`)
- Border: `1px solid var(--border)` (`#3D3226`)
- Radius: `var(--radius-md)` (8px)
- Shadow: `var(--shadow-md)` — `0 4px 12px rgba(0,0,0,0.5)`
- Padding: `16px`

### Inputs & Forms

**Search Input**
- Background: `var(--bg-base)` (`#1A1410`)
- Text: `var(--text-primary)`, 14px DM Sans
- Placeholder: `var(--text-faint)`
- Border: `1px solid var(--border)`
- Focus: `border-color: var(--teal); box-shadow: 0 0 0 3px var(--teal-glow)`
- Radius: `var(--radius-md)` (8px)
- Padding: `8px 14px 8px 36px` (left padding for search icon)

### Badges & Pills

**Kind Badge** (symbol types in search results, graph)
- Padding: `3px 8px`
- Radius: `4px`
- Font: Source Code Pro, 12px, weight 700, uppercase, `letter-spacing: 0.03em`
- Background: kind-specific color at 15% opacity
- Text: kind-specific color at full

**Tool Badge** (Inspector tool categories)
- Padding: `3px 8px`
- Radius: `10px` (pill)
- Font: Source Code Pro, 12px, weight 700
- Variants: `badge-search` (blue), `badge-nav` (purple), `badge-analysis` (copper), `badge-flow` (teal), `badge-context` (green)

### List Rows (session items, call rows, search results, references)
- Padding: `10px 16px`
- Min height: `40px`
- Border bottom: `1px solid var(--border)`
- Hover: `background: var(--bg-hover)`
- Active: `background: rgba(200, 145, 92, 0.08); border-left: 2px solid var(--copper)`
- Font: 14px for primary content, 13px mono for secondary data

### Navigation / Header
- Height: `52px`
- Background: `var(--bg-surface)`
- Border: `bottom 1px solid var(--border)`
- Logo: Playfair Display, 18px, weight 700, copper
- Path: Source Code Pro, 13px, muted text
- Stats: 13px, `var(--text-muted)` with `var(--text-secondary)` for values

### Code Viewer
- Font: Source Code Pro, 14px, line-height 1.6
- Line numbers: Source Code Pro, 12px, `var(--text-faint)`, 56px gutter width
- Highlight line: `background: rgba(200, 145, 92, 0.12); border-left: 3px solid var(--copper)`
- Background: `var(--bg-base)`

### JSON Block (Inspector detail)
- Font: Source Code Pro, 13px, line-height 1.6
- Background: `var(--bg-base)`
- Border: `1px solid var(--border)`
- Radius: `var(--radius-sm)` (4px)
- Padding: `10px 12px`
- Max height: `240px`, overflow-y auto

## 5. Layout Principles

### Spacing System
- Base unit: `4px`
- Scale: `0, 2, 4, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64`
- Primary rhythm: `8, 12, 16, 24` — most gaps and paddings use these four values

### Grid & Container
- Max content width: full viewport (edge-to-edge dashboard layout)
- Sidebar widths: 240px (session sidebar), 380px (detail panel)
- Gutter/gap: `8–16px` between flex items
- Page margins: none — chrome fills the viewport

### Whitespace Philosophy
- Dense but legible: pack information tightly but never sacrifice line-height or padding around text.
- Padding inside containers: minimum `12px`, standard `16px`. Never less than `10px`.
- Section separation: border + `12–16px` padding, not large blank gaps.

### Border Radius Scale

| Name | Value | Use |
|------|-------|-----|
| None | 0 | Graph edges, raw code blocks |
| Small | 4px | Buttons, badges, code blocks, inputs (compact) |
| Medium | 8px | Search input, cards, dropdowns |
| Large | 12px | Modals, large cards, search panel |
| XL | 16px | Hero containers (rare) |
| Pill | 9999px | Tool badges, concept tags |
| Circle | 50% | Avatar, graph node dots, status indicators |

## 6. Depth & Elevation

| Level | Treatment | Use |
|-------|-----------|-----|
| Flat (0) | No shadow, `var(--bg-base)` | Main content area, code viewer |
| Surface (1) | No shadow, `var(--bg-surface)` | Sidebars, headers, panels |
| Elevated (2) | `var(--shadow-md)` — `0 4px 12px rgba(0,0,0,0.5)` | Dropdowns, search results panel |
| Dialog (3) | `var(--shadow-lg)` — `0 8px 32px rgba(0,0,0,0.6)` | Slide-out detail panel, modals |
| Glow | `0 0 20px var(--copper-glow)` or `var(--teal-glow)` | Highlighted graph nodes, focus emphasis |

**Shadow Philosophy**: Depth is primarily communicated through background luminance stepping — darker = further back, lighter = closer. Shadows are dark and diffuse, reinforcing the warm cave-like atmosphere. Glow effects (copper, teal) are used sparingly for interactive emphasis on the graph canvas.

## 7. Do's and Don'ts

### Do
- Use `12px` as the absolute minimum text size — captions, line numbers, badges
- Use `13px` for labels, tabs, stat values, compact metadata, mono data
- Use `14px` as the default body size for descriptions, list items, references
- Use warm tones: copper for emphasis, teal for links, sage for success
- Use background luminance stepping for depth (deepest → base → surface → elevated)
- Use `Source Code Pro` for all technical data: paths, symbols, numbers, code, IDs
- Use `DM Sans` for all prose: labels, descriptions, headings, buttons

### Don't
- Never use text smaller than `12px` — no 9px, 10px, or 11px anywhere
- Never use pure white (`#FFFFFF`) or pure black (`#000000`) for text
- Never use cool-toned borders — all borders are warm (`#3D3226`, `#2E2820`)
- Never use more than 4 font weights (400, 500, 600, 700)
- Never apply copper or teal as large background fills — only as accents, borders, glows
- Never use `font-weight: bold` — use explicit numeric weights
- Never set `line-height` below `1.3` for any text

## 8. Responsive Behavior

### Breakpoints

| Name | Width | Key Changes |
|------|-------|-------------|
| Compact | <960px | Detail panel becomes overlay, sidebar collapses |
| Standard | 960–1440px | Default layout — sidebar + main + detail panel |
| Wide | >1440px | Detail panel and sidebar can be wider |

### Touch Targets
- Minimum interactive element: `32px x 32px`
- List row minimum height: `40px`
- Button minimum height: `36px`
- Adequate spacing between interactive elements: `4px` minimum gap

### Collapsing Strategy
- Sidebar: slides out as overlay on compact
- Detail panel: slides in from right as overlay (already does this)
- Tab bar: stays horizontal, no hamburger
- Graph canvas: fills available space

## 9. Agent Prompt Guide

### Quick Color Reference
- Primary CTA bg: `#C8915C`
- Page background: `#100D09`
- Panel surface: `#242019`
- Heading text: `#F0E6D6`
- Body text: `#B8A892`
- Muted text: `#8B7D6B`
- Border: `#3D3226`
- Link / teal: `#2D8F8F`
- Focus ring: `0 0 0 3px #2D8F8F33`

### Example Component Prompts
- "Create a sidebar section: `#242019` background, `1px solid #3D3226` border-right. Title at 13px DM Sans weight 700 uppercase `letter-spacing: 0.04em`, color `#5C5044`. List items at 14px, `#B8A892` text, `10px 16px` padding, hover `#38302A`."
- "Create a data table row: `10px 16px` padding, `1px solid #3D3226` bottom border. Label at 14px DM Sans `#B8A892`. Value at 13px Source Code Pro `#C8915C`. Hover `#38302A`."
- "Create a badge: `#242019` bg at 15% of category color, 12px Source Code Pro weight 700, `3px 8px` padding, `10px` border-radius."
- "Create a code block: `#1A1410` background, `1px solid #3D3226`, 4px radius, `10px 12px` padding. Font: 14px Source Code Pro, line-height 1.6, color `#B8A892`."

### Iteration Guide
1. Minimum text size is 12px — reject any design with smaller text
2. Labels and tabs: 13px uppercase weight 700 with 0.04em letter-spacing
3. Body content: 14px weight 400, mono data: 13–14px weight 400
4. Three accent colors: copper (emphasis), teal (interactive), sage (success) — everything else is neutral
5. All borders are `#3D3226` or `#2E2820` — warm, never gray or semi-transparent white
6. Padding inside any container: minimum 10px, standard 16px
7. Interactive row min-height: 40px with 10px vertical padding
