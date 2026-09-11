# Upstream attribution

This project derives from [EPUB Optimizer by HenryBaby](https://github.com/HenryBaby/epub-optimizer),
licensed under the GNU Affero General Public License v3.0 only.
The full license is in `LICENSE`.

Imported from upstream version 1.2.6, commit
`ea90b2e1d3897cf534f5ed1dabe29bb992a000b6`.

The processing modules in `src/epub_optimizer/` and the tests in
`test_core.py`, `test_epub_paths.py`, `test_epubcheck.py`,
`test_integration_optimize.py`, and `test_repair.py` come from that commit.
This adaptation adds CLI packaging and a command-line interface, and updates
package version reporting. It also annotates synchronous closure calls for lint.
One imported test was updated to account
for the engine wrapping blockquote text in paragraphs. It excludes the web
server and Docker setup.

EPUBCheck and Java are optional external programs and are not bundled.
