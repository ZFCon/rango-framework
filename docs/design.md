# Frontend colour scheme

Source of truth: [`crates/rango-views/assets/theme.css`](../crates/rango-views/assets/theme.css). The palette is rust-themed, matching the logo;
lizard green is the success colour.

| Role | Light | Dark |
|---|---|---|
| `--primary` (rust) | `#b7410e` | `#7c2d12` |
| `--secondary` (header, buttons, links) | `#8f3a14` | — |
| `--accent` (branding highlight) | `#f2c14e` | — |
| `--rango-rust-ink` (welcome page text, breadcrumbs bar) | `#5a2510` | `#f0a070` |
| `--link-fg` | `#8f3a14` | `#f0a070` |
| `--error-fg` (crimson, distinct from rust) | `#b4172b` | `#ff7a85` |
| success tint / `--rango-lizard` | `#dff0d8` / `#43a047` | `#1f5a2a` |

## Where it applies

- **Admin:** standard CSS variable names (`--primary`, `--secondary`, ...) are used by the admin stylesheets;
  load `theme.css` after `base.css` (and it also covers `dark_mode.css`'s selectors).
- **Debug pages, welcome page, default error views (M2):** their templates reference the
  `--rango-*` tokens.
- **Form/widget CSS (M6):** use the same variables; no new colours.

All text/background pairs have WCAG contrast of at least 4.5:1 (checked when the palette was chosen).
