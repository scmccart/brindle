//! The tab strip's active tab, apart from GPUI: which tab a new one goes
//! after, which tab is active once one is removed, switched to or moved.
//!
//! `Workspace` keeps its tabs in one `Vec`, which is the strip's order. Each
//! method here takes that order as the tabs' ids, gathered for the call, and
//! the model keeps the active tab by its id, so no position is kept across a
//! change (`types R3`). An index a method returns is valid only within the
//! call that asked for it.

/// A tab's identity for the life of the window: what the strip's triggers,
/// a tab menu and a notification name it by, wherever it moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// The tab `ActivateTab` goes to: the nth from the left (from 0; alt-1 to
/// alt-8, `activate_tab_N`), or the last whatever its number (`last_tab`,
/// alt-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabTarget {
    Nth(usize),
    Last,
}

/// The tab [`Tabs::pick`] makes active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// That tab (a press on its face, a notification clicked).
    Tab(TabId),
    Target(TabTarget),
    /// The one after the active tab, the first after the last.
    Next,
    /// The one before the active tab, the last before the first.
    Prev,
}

/// How far [`Tabs::shift`] moves a tab, and what happens at an end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    /// Past one end it comes in at the other (`MoveTabLeft`, `MoveTabRight`).
    Wrapping(isize),
    /// Past an end it doesn't move (the tab menu's moves).
    Within(isize),
}

/// What is left once a tab is removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removed {
    /// It was the last tab: the window closes.
    Last,
    /// The tab now active, which the window shows and focuses.
    Active(TabId),
}

/// The strip's active tab, by id, and the order tabs were last active in.
#[derive(Debug, Default)]
pub struct Tabs {
    active: Option<TabId>,
    /// Each tab that has been active, the most recent last (OpenSpec
    /// change `agents-prompt-context`: which agent zone a terminal's
    /// output is offered to first).
    shown: Vec<TabId>,
}

impl Tabs {
    /// A strip whose active tab is `active`, as a test sets one up.
    #[cfg(test)]
    fn with(active: Option<TabId>) -> Self {
        let mut tabs = Self::default();
        tabs.set_active(active);
        tabs
    }

    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    fn set_active(&mut self, active: Option<TabId>) {
        self.active = active;
        if let Some(tab) = active {
            self.shown.retain(|shown| *shown != tab);
            self.shown.push(tab);
        }
    }

    /// How recently `tab` was the active one: larger is more recent, the
    /// active tab the largest, and 0 for one never active.
    pub fn recency(&self, tab: TabId) -> u64 {
        self.shown.iter().position(|shown| *shown == tab).map_or(0, |at| at as u64 + 1)
    }

    /// The active tab's place in `order`; `None` with no tabs.
    pub fn active_in(&self, order: &[TabId]) -> Option<usize> {
        let active = self.active?;
        order.iter().position(|id| *id == active)
    }

    /// Where `tab` goes in `order`: at `requested` (clamped to the end), else
    /// right after the active tab, else first. The active tab stays the
    /// same, except that the first tab of an empty strip becomes the active
    /// one: a window always shows a tab when it has one.
    pub fn insert_at(&mut self, order: &[TabId], tab: TabId, requested: Option<usize>) -> usize {
        let after_active = self.active_in(order).map_or(0, |at| at + 1);
        if self.active.is_none() {
            self.set_active(Some(tab));
        }
        requested.unwrap_or(after_active).min(order.len())
    }

    /// `removed` leaves `order_before`. The active tab stays the same,
    /// unless it is the one removed: then the tab after it takes over, else
    /// (it was the last) the one before. `Removed::Last` when no tab is left.
    pub fn removed(&mut self, order_before: &[TabId], removed: TabId) -> Removed {
        let at = order_before.iter().position(|id| *id == removed).unwrap_or(0);
        let after: Vec<TabId> = order_before.iter().copied().filter(|id| *id != removed).collect();
        self.shown.retain(|shown| *shown != removed);
        let Some(last) = after.len().checked_sub(1) else {
            self.set_active(None);
            return Removed::Last;
        };
        let active = match self.active.filter(|active| after.contains(active)) {
            Some(active) => active,
            None => after[at.min(last)],
        };
        self.set_active(Some(active));
        Removed::Active(active)
    }

    /// Makes the tab `pick` names in `order` the active one, and returns it;
    /// `None`, changing nothing, when there is no such tab (a number past
    /// the last tab, no tabs).
    pub fn pick(&mut self, order: &[TabId], pick: Pick) -> Option<TabId> {
        let len = order.len();
        let at = match pick {
            Pick::Tab(tab) => order.iter().position(|id| *id == tab),
            Pick::Target(TabTarget::Nth(n)) => Some(n),
            Pick::Target(TabTarget::Last) => len.checked_sub(1),
            Pick::Next => (len > 0).then(|| self.active_in(order).map_or(0, |at| (at + 1) % len)),
            Pick::Prev => (len > 0).then(|| self.active_in(order).map_or(0, |at| (at + len - 1) % len)),
        }?;
        let tab = *order.get(at)?;
        self.set_active(Some(tab));
        Some(tab)
    }

    /// The move of tab `at` by `by` in `order`, as `(from, to)` places for
    /// `reorder`: the tab takes `to`'s place. `None` when it doesn't move (a
    /// lone tab, a move past an end that doesn't wrap, a tab not there). The
    /// active tab stays the same, wherever it ends up.
    pub fn shift(&self, order: &[TabId], at: TabId, by: Shift) -> Option<(usize, usize)> {
        let from = order.iter().position(|id| *id == at)?;
        let len = order.len();
        let to = match by {
            Shift::Wrapping(_) if len < 2 => return None,
            Shift::Wrapping(delta) => (from as isize + delta).rem_euclid(len as isize) as usize,
            Shift::Within(delta) => from.checked_add_signed(delta).filter(|to| *to < len)?,
        };
        (to != from).then_some((from, to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strip as `Workspace` kept it before this model: positions, and
    /// the active tab as an index every change patched. Each method is the
    /// workspace's code of the time, on ids alone, so the model is checked
    /// against it on every order of up to four tabs.
    #[derive(Clone, Debug)]
    struct Before {
        ids: Vec<u64>,
        active: usize,
    }

    impl Before {
        /// `insert_tab_inactive`.
        fn insert_inactive(&mut self, id: u64, index: Option<usize>) -> usize {
            let index = index.unwrap_or(if self.ids.is_empty() { 0 } else { self.active + 1 }).min(self.ids.len());
            if !self.ids.is_empty() && index <= self.active {
                self.active += 1;
            }
            self.ids.insert(index, id);
            index
        }

        /// `select`, which `activate` calls.
        fn select(&mut self, index: usize) -> bool {
            if self.ids.is_empty() {
                return false;
            }
            self.active = index.min(self.ids.len() - 1);
            true
        }

        /// `remove_tab`: the active index once `ix` is gone, or `None` for
        /// the last tab (the window closes).
        fn remove(&mut self, ix: usize) -> Option<usize> {
            self.ids.remove(ix);
            if self.ids.is_empty() {
                return None;
            }
            if self.active > ix || self.active >= self.ids.len() {
                self.active = self.active.saturating_sub(1);
            }
            self.select(self.active);
            Some(self.active)
        }

        fn next(&mut self) {
            if !self.ids.is_empty() {
                self.select((self.active + 1) % self.ids.len());
            }
        }

        fn prev(&mut self) {
            if !self.ids.is_empty() {
                self.select((self.active + self.ids.len() - 1) % self.ids.len());
            }
        }

        /// `activate_tab`, `usize::MAX` standing for the last tab.
        fn activate_tab(&mut self, n: usize) {
            let ix = if n == usize::MAX { self.ids.len().saturating_sub(1) } else { n };
            if ix < self.ids.len() {
                self.select(ix);
            }
        }

        /// `move_tab`.
        fn move_tab(&mut self, delta: isize) {
            let len = self.ids.len() as isize;
            if len < 2 {
                return;
            }
            let to = (self.active as isize + delta).rem_euclid(len) as usize;
            let tab = self.ids.remove(self.active);
            self.ids.insert(to, tab);
            self.active = to;
        }

        /// `move_tab_at`, through `reorder_keeping`.
        fn move_tab_at(&mut self, ix: usize, delta: isize) {
            let Some(to) = ix.checked_add_signed(delta) else { return };
            let Some(&active_id) = self.ids.get(self.active) else { return };
            if to == ix || ix >= self.ids.len() || to >= self.ids.len() {
                return;
            }
            let item = self.ids.remove(ix);
            self.ids.insert(to, item);
            self.active = self.ids.iter().position(|id| *id == active_id).unwrap_or(0);
        }

        fn active_id(&self) -> Option<TabId> {
            self.ids.get(self.active).map(|id| TabId(*id))
        }

        fn order(&self) -> Vec<TabId> {
            self.ids.iter().map(|id| TabId(*id)).collect()
        }
    }

    /// Every strip of 0 to 4 tabs (ids 10, 11, …), with each tab active in
    /// turn (an empty one with none), as it was kept and as the model keeps it.
    fn strips() -> Vec<(Before, Tabs)> {
        let mut strips = Vec::new();
        for len in 0..=4u64 {
            let ids: Vec<u64> = (10..10 + len).collect();
            for active in 0..(len as usize).max(1) {
                let before = Before { ids: ids.clone(), active };
                let tabs = Tabs::with(before.active_id());
                strips.push((before, tabs));
            }
        }
        strips
    }

    /// A tab inserted where the strip of the time put it, with the same
    /// tab active after, whether it is activated or not. The first tab of
    /// an empty strip is the active one.
    #[test]
    fn a_tab_is_inserted_where_it_was() {
        for (before, tabs) in strips() {
            for requested in [None, Some(0), Some(1), Some(2), Some(4), Some(9)] {
                let (mut was, mut now, new) = (before.clone(), Tabs::with(tabs.active), TabId(99));
                let order = was.order();
                let at = now.insert_at(&order, new, requested);
                assert_eq!(at, was.insert_inactive(99, requested), "{before:?} at {requested:?}");
                assert_eq!(now.active(), was.active_id(), "{before:?} at {requested:?}, inactive");
                let mut order = order;
                order.insert(at, new);
                // `insert_tab`: then activated.
                was.select(at);
                assert_eq!(now.pick(&order, Pick::Tab(new)), was.active_id());
            }
        }
        // The table's rows, spelled out.
        let order = [TabId(10), TabId(11), TabId(12)];
        let mut tabs = Tabs::with(Some(TabId(11)));
        assert_eq!(tabs.insert_at(&order, TabId(99), None), 2, "after the active tab");
        assert_eq!(tabs.insert_at(&order, TabId(99), Some(3)), 3, "at the end");
        assert_eq!(tabs.insert_at(&order, TabId(99), Some(7)), 3, "clamped to the end");
        assert_eq!(tabs.active(), Some(TabId(11)));
        let mut empty = Tabs::default();
        assert_eq!(empty.insert_at(&[], TabId(5), Some(3)), 0);
        assert_eq!(empty.active(), Some(TabId(5)), "the first tab is shown");
    }

    /// The active tab once one is removed: the same tab, or after the
    /// active one's removal the tab after it, else the one before.
    #[test]
    fn the_active_tab_after_a_removal_is_the_one_it_was() {
        for (before, tabs) in strips() {
            for ix in 0..before.ids.len() {
                let (mut was, mut now) = (before.clone(), Tabs::with(tabs.active));
                let order = was.order();
                let removed = now.removed(&order, order[ix]);
                let expected = match was.remove(ix) {
                    None => Removed::Last,
                    Some(_) => Removed::Active(was.active_id().expect("a tab is left")),
                };
                assert_eq!(removed, expected, "{before:?} without {ix}");
                assert_eq!(now.active(), was.active_id());
            }
        }
        let order = [TabId(10), TabId(11), TabId(12)];
        let removing = |active: u64, removed: u64| Tabs::with(Some(TabId(active))).removed(&order, TabId(removed));
        assert_eq!(removing(11, 11), Removed::Active(TabId(12)), "the one after");
        assert_eq!(removing(12, 12), Removed::Active(TabId(11)), "the last: the one before");
        assert_eq!(removing(10, 12), Removed::Active(TabId(10)), "another's removal leaves it");
        assert_eq!(removing(12, 10), Removed::Active(TabId(12)));
        let mut lone = Tabs::with(Some(TabId(10)));
        assert_eq!(lone.removed(&[TabId(10)], TabId(10)), Removed::Last);
        assert_eq!(lone.active(), None);
    }

    /// Next, previous and a tab by number or as the last, as the strip's
    /// keys and `ActivateTab` switched them; a number past the last tab and
    /// an empty strip change nothing.
    #[test]
    fn picks_switch_as_they_did() {
        for (before, tabs) in strips() {
            let order = before.order();
            let check = |pick: Pick, was: &dyn Fn(&mut Before)| {
                let (mut was_strip, mut now) = (before.clone(), Tabs::with(tabs.active));
                was(&mut was_strip);
                let picked = now.pick(&order, pick);
                assert_eq!(now.active(), was_strip.active_id(), "{before:?}: {pick:?}");
                if picked.is_some() {
                    assert_eq!(picked, was_strip.active_id(), "{before:?}: {pick:?}");
                }
            };
            check(Pick::Next, &Before::next);
            check(Pick::Prev, &Before::prev);
            check(Pick::Target(TabTarget::Last), &|was: &mut Before| was.activate_tab(usize::MAX));
            for n in 0..6 {
                check(Pick::Target(TabTarget::Nth(n)), &|was: &mut Before| was.activate_tab(n));
            }
            for (ix, id) in order.iter().enumerate() {
                check(Pick::Tab(*id), &|was: &mut Before| {
                    was.select(ix);
                });
            }
        }
        let order = [TabId(10), TabId(11), TabId(12)];
        let mut tabs = Tabs::with(Some(TabId(12)));
        assert_eq!(tabs.pick(&order, Pick::Next), Some(TabId(10)), "wraps to the first");
        assert_eq!(tabs.pick(&order, Pick::Prev), Some(TabId(12)), "wraps to the last");
        assert_eq!(tabs.pick(&order, Pick::Target(TabTarget::Nth(3))), None, "no fourth tab");
        assert_eq!(tabs.active(), Some(TabId(12)));
        assert_eq!(tabs.pick(&order, Pick::Target(TabTarget::Nth(0))), Some(TabId(10)));
        assert_eq!(tabs.pick(&order, Pick::Target(TabTarget::Last)), Some(TabId(12)));
        assert_eq!(tabs.pick(&order, Pick::Tab(TabId(77))), None, "a tab gone meanwhile");
        assert_eq!(Tabs::default().pick(&[], Pick::Next), None);
        assert_eq!(Tabs::default().pick(&[], Pick::Target(TabTarget::Last)), None);
    }

    /// A move of the active tab wraps; a tab menu's move of any tab
    /// doesn't. Either way the active tab stays the one it was.
    #[test]
    fn shifts_move_tabs_as_they_did() {
        let moved = |order: &[TabId], (from, to): (usize, usize)| {
            let mut order = order.to_vec();
            let tab = order.remove(from);
            order.insert(to, tab);
            order
        };
        for (before, tabs) in strips() {
            let order = before.order();
            for delta in [-1, 1] {
                // `move_tab`: the active tab, wrapping.
                let mut was = before.clone();
                was.move_tab(delta);
                let shifted = tabs.active().and_then(|active| tabs.shift(&order, active, Shift::Wrapping(delta)));
                let after = shifted.map_or_else(|| order.clone(), |places| moved(&order, places));
                assert_eq!(after, was.order(), "{before:?} by {delta}");
                assert_eq!(tabs.active_in(&after).map(|at| after[at]), was.active_id());
                // `move_tab_at`: any tab, within the ends.
                for ix in 0..order.len() {
                    let mut was = before.clone();
                    was.move_tab_at(ix, delta);
                    let shifted = tabs.shift(&order, order[ix], Shift::Within(delta));
                    let after = shifted.map_or_else(|| order.clone(), |places| moved(&order, places));
                    assert_eq!(after, was.order(), "{before:?}: {ix} by {delta}");
                    assert_eq!(tabs.active_in(&after).map(|at| after[at]), was.active_id());
                }
            }
        }
        let order = [TabId(10), TabId(11), TabId(12)];
        let tabs = Tabs::with(Some(TabId(12)));
        assert_eq!(tabs.shift(&order, TabId(12), Shift::Wrapping(1)), Some((2, 0)), "the last comes in first");
        assert_eq!(tabs.shift(&order, TabId(10), Shift::Wrapping(-1)), Some((0, 2)));
        assert_eq!(tabs.shift(&order, TabId(12), Shift::Within(1)), None, "not past the end");
        assert_eq!(tabs.shift(&order, TabId(10), Shift::Within(-1)), None);
        assert_eq!(tabs.shift(&order, TabId(11), Shift::Within(1)), Some((1, 2)));
        assert_eq!(tabs.shift(&[TabId(10)], TabId(10), Shift::Wrapping(1)), None, "a lone tab");
        assert_eq!(tabs.shift(&order, TabId(77), Shift::Within(1)), None, "a tab gone meanwhile");
    }

    /// Recency follows activation: the active tab the most recent, a tab
    /// never active 0, and a removed tab forgotten.
    #[test]
    fn tabs_are_ranked_by_when_they_were_last_active() {
        let order = [TabId(10), TabId(11), TabId(12)];
        let mut tabs = Tabs::default();
        tabs.insert_at(&[], TabId(10), None);
        assert_eq!(tabs.recency(TabId(10)), 1);
        assert_eq!(tabs.recency(TabId(11)), 0, "never active");
        tabs.pick(&order, Pick::Tab(TabId(12)));
        tabs.pick(&order, Pick::Tab(TabId(11)));
        tabs.pick(&order, Pick::Tab(TabId(10)));
        let ranked = |tabs: &Tabs| order.map(|tab| tabs.recency(tab));
        assert_eq!(ranked(&tabs), [3, 2, 1]);
        tabs.pick(&order, Pick::Tab(TabId(12)));
        assert_eq!(ranked(&tabs), [2, 1, 3]);
        assert_eq!(tabs.removed(&order, TabId(12)), Removed::Active(TabId(11)), "the one before the last");
        assert_eq!(ranked(&tabs), [1, 2, 0]);
    }

    #[test]
    fn the_active_tab_is_found_by_id() {
        let order = [TabId(10), TabId(11), TabId(12)];
        assert_eq!(Tabs::with(Some(TabId(11))).active_in(&order), Some(1));
        assert_eq!(Tabs::with(Some(TabId(77))).active_in(&order), None);
        assert_eq!(Tabs::default().active_in(&order), None);
        assert_eq!(Tabs::default().active_in(&[]), None);
    }
}
