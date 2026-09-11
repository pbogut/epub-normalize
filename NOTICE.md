# Upstream attribution

This project derives from [EPUB Optimizer by HenryBaby](https://github.com/HenryBaby/epub-optimizer),
licensed under the GNU Affero General Public License v3.0 only.
The full license is in `LICENSE`.

Imported from upstream version 1.2.6, commit
`ea90b2e1d3897cf534f5ed1dabe29bb992a000b6`.

The Rust modules in `src/` port the original Python processing algorithms,
including typography classification, archive handling, validation, and repairs.
`src/canonical.css` retains the upstream stylesheet. The test fixtures derive
from the upstream tests and this project's Python implementation at commit
`1950cf9`.

This adaptation adds a command-line interface, Calibre integration, local
hyperlink repair before validation, and atomic output publication. The Rust
rewrite replaces the Python runtime and packaging. It retains the upstream
stylesheet, metadata marker, and report names. The upstream web server and
Docker setup are not included.

EPUBCheck and Java are optional external programs and are not bundled.
