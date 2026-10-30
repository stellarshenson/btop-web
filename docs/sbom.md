# SBOM

`sbom.cdx.json` in the repository root is a CycloneDX 1.6 JSON SBOM. `make sbom` regenerates it offline from `Cargo.lock` and the manifests.

## Regenerate

- Command: `make sbom`
- Script: `scripts/sbom.py`, Python 3 standard library only
- Inputs: `Cargo.lock`; the workspace manifests through `cargo metadata --offline --locked --no-deps`; each registry crate's own `Cargo.toml` under `$CARGO_HOME/registry/src/<name>-<version>/`
- Network: none; the crate sources must be in the local cargo cache, which any build or `cargo fetch` populates
- Determinism: components are sorted and no timestamp or serial number is written, so two runs are byte-identical

## Component counts

- 180 components: 1 application, 168 library, 11 file
- 169 crates in `Cargo.lock`, each appears exactly once: `btop-web` is the application, the other 168 are libraries
- Each crate component carries the name and version from `Cargo.lock`, the purl `pkg:cargo/<name>@<version>`, and the licence read from that crate's own manifest `license` field
- `btop-web 0.6.30` is MIT, inherited from `[workspace.package].license` in the root `Cargo.toml`

## Licence syntax

Cargo's legacy `/` separator in a licence field means the SPDX `OR` disjunction. Ten crates use it; the SBOM carries the SPDX form:

| Crate | Manifest value | SBOM expression |
|---|---|---|
| cexpr 0.6.0 | Apache-2.0/MIT | Apache-2.0 OR MIT |
| fnv 1.0.7 | Apache-2.0 / MIT | Apache-2.0 OR MIT |
| ident_case 1.0.1 | MIT/Apache-2.0 | MIT OR Apache-2.0 |
| lazycell 1.3.0 | MIT/Apache-2.0 | MIT OR Apache-2.0 |
| minimal-lexical 0.2.1 | MIT/Apache-2.0 | MIT OR Apache-2.0 |
| rustc-hash 1.1.0 | Apache-2.0/MIT | Apache-2.0 OR MIT |
| same-file 1.0.6 | Unlicense/MIT | Unlicense OR MIT |
| serde_urlencoded 0.7.1 | MIT/Apache-2.0 | MIT OR Apache-2.0 |
| walkdir 2.5.0 | Unlicense/MIT | Unlicense OR MIT |
| wrapcenum-derive 0.4.1 | MIT/Apache-2.0 | MIT OR Apache-2.0 |

Every other value is copied from the manifest unchanged, as a single SPDX id (`MIT`, `Apache-2.0`, `BSD-3-Clause`, `ISC`, `Zlib`) or an SPDX expression.

## Entries that are not crates

These 11 components are files the repository ships but Cargo does not build, so they are not in `Cargo.lock`. Each carries type `file`, a version, its licence and a `path` property.

| Component | Version | Licence | Path |
|---|---|---|---|
| GalaxaLabsTron Narrow | 1.000 | OFL-1.1 | crates/btop-web/assets/fonts/GalaxaLabsTronNarrow-VF.ttf |
| htmx | 2.0.11 | BSD-2-Clause | crates/btop-web/assets/htmx.min.js |
| htmx-ext-sse | 2.2.2 | BSD-2-Clause | crates/btop-web/assets/htmx-sse.min.js |
| everforest-dark-medium.theme | 1.3.0 | Apache-2.0 | crates/btop-web/themes/everforest-dark-medium.theme |
| everforest-light-medium.theme | 1.3.0 | Apache-2.0 | crates/btop-web/themes/everforest-light-medium.theme |
| flexoki-dark.theme | 1.3.0 | Apache-2.0 | crates/btop-web/themes/flexoki-dark.theme |
| flexoki-light.theme | 1.3.0 | Apache-2.0 | crates/btop-web/themes/flexoki-light.theme |
| gruvbox_dark.theme | 1.3.0 | Apache-2.0 | crates/btop-web/themes/gruvbox_dark.theme |
| gruvbox_light.theme | 1.3.0 | Apache-2.0 | crates/btop-web/themes/gruvbox_light.theme |
| solarized_dark.theme | 1.3.0 | Apache-2.0 | crates/btop-web/themes/solarized_dark.theme |
| solarized_light.theme | 1.3.0 | Apache-2.0 | crates/btop-web/themes/solarized_light.theme |

- The font is OFL-1.1, derived from Orbitron; `1.000` is the version in name ID 5 of the font, and the licence text is `crates/btop-web/assets/fonts/GalaxaLabsTronNarrow-OFL.txt`
- htmx keeps `version: "2.0.11"` in the vendored file; htmx-ext-sse 2.2.2 is recorded in `NOTICE.md`; both licences are BSD-2-Clause per `NOTICE.md`
- The eight theme files are btop 1.3.0 files; their `Apache-2.0` value is read from each file's `# Licence:` header
- The other eight theme files (cyberpunk, synthwave, terracotta and ultraviolet pairs) are btop-web originals, not btop files; they are covered by the application component's MIT licence

## NOASSERTION

- None on this tree: every crate manifest was readable and named its licence
- Rule: a crate whose manifest is missing from the cargo cache, or whose manifest has no `license` field, is listed with the expression `NOASSERTION`; `make sbom` prints its name and version, and the reason belongs in this section
