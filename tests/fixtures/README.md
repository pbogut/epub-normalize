# Compatibility fixtures

The input EPUBs and `compatibility.json` capture the Python implementation at
commit `1950cf9`. Each of the 24 archive fixtures runs with publisher CSS removal
and preservation, for 48 cases. The expected results include every output archive
entry and the preview report. The canonical stylesheet is copied byte-for-byte.

Rust tests compare XML structure, attributes, and text, allowing serialization
differences such as attribute order and namespace prefix spelling. Other resources
and the embedded JSON reports are compared directly. These fixtures need no Python
runtime to execute.
