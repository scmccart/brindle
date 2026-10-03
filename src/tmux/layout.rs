//! Parser for tmux window layout strings, e.g.
//! `95e4,120x40,0,0{60x40,0,0,0,59x40,61,0[59x20,61,0,1,59x19,61,21,2]}`.
//!
//! `{…}` children are side by side, `[…]` children are stacked.

use super::protocol::PaneId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    Pane { id: PaneId, rect: Rect },
    /// Side by side (`{}`).
    Columns { rect: Rect, children: Vec<Layout> },
    /// Stacked (`[]`).
    Rows { rect: Rect, children: Vec<Layout> },
}

impl Layout {
    pub fn rect(&self) -> Rect {
        match self {
            Layout::Pane { rect, .. } | Layout::Columns { rect, .. } | Layout::Rows { rect, .. } => *rect,
        }
    }

    /// All panes, in layout order.
    pub fn panes(&self) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Layout::Pane { id, rect } => out.push((*id, *rect)),
            Layout::Columns { children, .. } | Layout::Rows { children, .. } => {
                for child in children {
                    child.collect(out);
                }
            }
        }
    }

    /// Divider lines between siblings, as cell rectangles one cell thick.
    pub fn dividers(&self) -> Vec<Rect> {
        let mut out = Vec::new();
        self.collect_dividers(&mut out);
        out
    }

    fn collect_dividers(&self, out: &mut Vec<Rect>) {
        match self {
            Layout::Pane { .. } => {}
            Layout::Columns { rect, children } => {
                for pair in children.windows(2) {
                    let left = pair[0].rect();
                    out.push(Rect { x: left.x + left.width, y: rect.y, width: 1, height: rect.height });
                }
                children.iter().for_each(|c| c.collect_dividers(out));
            }
            Layout::Rows { rect, children } => {
                for pair in children.windows(2) {
                    let top = pair[0].rect();
                    out.push(Rect { x: rect.x, y: top.y + top.height, width: rect.width, height: 1 });
                }
                children.iter().for_each(|c| c.collect_dividers(out));
            }
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn number(&mut self) -> Option<u32> {
        let start = self.pos;
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        std::str::from_utf8(&self.s[start..self.pos]).ok()?.parse().ok()
    }

    fn expect(&mut self, c: u8) -> Option<()> {
        (self.s.get(self.pos) == Some(&c)).then(|| self.pos += 1)
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn cell(&mut self) -> Option<Layout> {
        let width = self.number()? as u16;
        self.expect(b'x')?;
        let height = self.number()? as u16;
        self.expect(b',')?;
        let x = self.number()? as u16;
        self.expect(b',')?;
        let y = self.number()? as u16;
        let rect = Rect { x, y, width, height };
        match self.peek() {
            Some(b',') => {
                // Either a pane id, or (in a parent list) the next sibling;
                // a pane id is followed by `,`, `}`, `]` or the end — never `x`.
                let save = self.pos;
                self.pos += 1;
                let id = self.number();
                if let Some(id) = id
                    && self.peek() != Some(b'x')
                {
                    return Some(Layout::Pane { id, rect });
                }
                self.pos = save;
                None
            }
            Some(open @ (b'{' | b'[')) => {
                self.pos += 1;
                let close = if open == b'{' { b'}' } else { b']' };
                let mut children = vec![self.cell()?];
                while self.peek() == Some(b',') {
                    self.pos += 1;
                    children.push(self.cell()?);
                }
                self.expect(close)?;
                Some(if open == b'{' {
                    Layout::Columns { rect, children }
                } else {
                    Layout::Rows { rect, children }
                })
            }
            _ => None,
        }
    }
}

/// Parses a full layout string including its leading checksum.
pub fn parse(layout: &str) -> Option<Layout> {
    let (_checksum, body) = layout.split_once(',')?;
    let mut parser = Parser { s: body.as_bytes(), pos: 0 };
    let tree = parser.cell()?;
    (parser.pos == parser.s.len()).then_some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn single_pane() {
        assert_eq!(parse("aafd,120x40,0,0,0"), Some(Layout::Pane { id: 0, rect: r(0, 0, 120, 40) }));
    }

    #[test]
    fn side_by_side() {
        let layout = parse("f91d,120x40,0,0{60x40,0,0,0,59x40,61,0,1}").unwrap();
        assert_eq!(layout.panes(), vec![(0, r(0, 0, 60, 40)), (1, r(61, 0, 59, 40))]);
        assert_eq!(layout.dividers(), vec![r(60, 0, 1, 40)]);
    }

    #[test]
    fn nested() {
        let layout =
            parse("95e4,120x40,0,0{60x40,0,0,0,59x40,61,0[59x20,61,0,1,59x19,61,21,2]}").unwrap();
        assert_eq!(
            layout.panes(),
            vec![(0, r(0, 0, 60, 40)), (1, r(61, 0, 59, 20)), (2, r(61, 21, 59, 19))]
        );
        assert_eq!(layout.dividers(), vec![r(60, 0, 1, 40), r(61, 20, 59, 1)]);
    }

    #[test]
    fn large_ids_and_deep_nesting() {
        let layout = parse(
            "bb62,159x48,0,0{79x48,0,0[79x24,0,0,12,79x23,0,25{39x23,0,25,13,39x23,40,25,14}],79x48,80,0,15}",
        )
        .unwrap();
        let ids: Vec<_> = layout.panes().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![12, 13, 14, 15]);
    }

    #[test]
    fn garbage() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("abcd,120x40"), None);
        assert_eq!(parse("abcd,120x40,0,0{60x40,0,0,0"), None);
        assert_eq!(parse("abcd,120x40,0,0,0trailing"), None);
    }
}
