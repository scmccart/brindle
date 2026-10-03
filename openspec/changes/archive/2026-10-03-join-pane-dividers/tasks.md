# Tasks

## 1. Divider junctions

- [x] 1.1 Add a pure `divider_edge` in `src/tmux_view.rs`, which places each segment end at a cell edge or on a crossing divider's line, and use it in `paint_dividers`. Unit-test the T-junction and 2x2 grid layouts, and verify with `cargo test tmux_view`.
- [x] 1.2 Capture a T-junction window and a 2x2 tiled window under Xwayland on a throwaway tmux server, and confirm in enlarged crops that the lines meet and the highlight turns its corners on the lines.
