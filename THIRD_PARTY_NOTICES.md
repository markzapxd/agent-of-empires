# Third-party notices

Agent of Empires is MIT licensed (see `LICENSE`). It also contains a small
amount of code derived from third-party projects under their own terms, listed
here as those licenses require.

## React Flow

- Project: <https://github.com/xyflow/xyflow>
- Packages: `@xyflow/react` and `@xyflow/system`
- Used in: the web dashboard's Agent Graph
- License: MIT

Copyright (c) 2019-2025 webkid GmbH

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## herdr

- Project: <https://github.com/herdrdev/herdr>
- License: Apache License 2.0, full text in [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt)
- Used in: `src/tui/hyperlink.rs`

The OSC 8 hyperlink state machine in `src/tui/hyperlink.rs` is derived from
herdr's `src/protocol/render_ansi.rs`: emitting a sequence only when the target
changes between cells, resetting to a known hyperlink state at the start of
every frame, and stripping control bytes from a target before writing it.

Modifications: herdr blits whole frames from its own wire format, in which each
cell carries an index into a per-frame hyperlink table, and it owns the output
writer outright. The version here wraps ratatui's `CrosstermBackend` and
resolves a cell's target through a side map keyed by position, because aoe's
grid comes from `vt100`, which does not model hyperlinks per cell. The
surrounding backend, the map, and the tests are original.
