//! The layout a quitting Brindle detaches each tab's terminal server
//! sessions with, and how a launch's sweep puts sessions back into their tabs
//! from it (OpenSpec change `session-layout-in-server`).
//!
//! Every session of a tab is detached with the same blob: the tab's split
//! tree with session keys as leaves, its active zone, its manual title and
//! its place among the quit's tabs ([`record`]). The core keeps it in the
//! session's meta without reading it and lists it with the session while it
//! stays detached. A sweep groups the sessions it opens by the tab they name
//! ([`group`]), so a split comes back whichever Brindle sweeps it.
//!
//! `session.toml` still restores the instance's own tabs first; only the
//! sessions it doesn't take are grouped here.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::remote::protocol::client::SessionInfo;
use crate::zone_view::{SplitAxis, ZoneId, ZoneTree};

/// The blob format this build writes and reads; another is ignored.
pub const VERSION: u32 = 1;

/// One tab, as each of its sessions carries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub v: u32,
    /// The quit that recorded it ([`quit_id`]); with `index`, the tab.
    pub quit: String,
    /// The tab's place in the strip at that quit.
    pub index: u32,
    /// Its manual title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub zones: Node,
}

/// A zone split tree with session keys as leaves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Split {
        split: SplitAxis,
        first: Box<Node>,
        second: Box<Node>,
    },
    Leaf {
        key: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        active: bool,
    },
}

impl Node {
    fn has_key(&self, wanted: &str) -> bool {
        match self {
            Node::Leaf { key, .. } => key == wanted,
            Node::Split { first, second, .. } => first.has_key(wanted) || second.has_key(wanted),
        }
    }
}

/// An id for one quit of one Brindle, shared by the layouts of all its
/// tabs: unique enough to keep two quits' tabs apart.
pub fn quit_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{:x}-{nanos:x}", std::process::id())
}

/// The layout a tab's sessions are detached with: `tree` with each zone's
/// session key from `key_of`. A zone without one (a message, a session not
/// started yet) is left out, its split's sibling taking its place; the first
/// zone left is active when `active` was left out. `None` when no zone has a
/// session.
pub fn record(
    tree: &ZoneTree,
    active: ZoneId,
    key_of: &mut impl FnMut(ZoneId) -> Option<String>,
    quit: &str,
    index: usize,
    title: Option<&str>,
) -> Option<serde_json::Value> {
    fn node(tree: &ZoneTree, active: ZoneId, key_of: &mut impl FnMut(ZoneId) -> Option<String>) -> Option<Node> {
        match tree {
            ZoneTree::Leaf(id) => key_of(*id).map(|key| Node::Leaf { key, active: *id == active }),
            ZoneTree::Split { axis, first, second } => {
                match (node(first, active, key_of), node(second, active, key_of)) {
                    (Some(first), Some(second)) => {
                        Some(Node::Split { split: *axis, first: Box::new(first), second: Box::new(second) })
                    }
                    (one, other) => one.or(other),
                }
            }
        }
    }
    let mut zones = node(tree, active, key_of)?;
    if !node_has_active(&zones) {
        activate_first(&mut zones);
    }
    let layout = Layout {
        v: VERSION,
        quit: quit.to_string(),
        index: u32::try_from(index).unwrap_or(u32::MAX),
        title: title.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string),
        zones,
    };
    serde_json::to_value(layout).ok()
}

fn node_has_active(node: &Node) -> bool {
    match node {
        Node::Leaf { active, .. } => *active,
        Node::Split { first, second, .. } => node_has_active(first) || node_has_active(second),
    }
}

fn activate_first(node: &mut Node) {
    match node {
        Node::Leaf { active, .. } => *active = true,
        Node::Split { first, .. } => activate_first(first),
    }
}

/// A zone split tree with `T` at its leaves.
#[derive(Debug, Clone, PartialEq)]
pub enum Tree<T> {
    Leaf { item: T, active: bool },
    Split { axis: SplitAxis, first: Box<Tree<T>>, second: Box<Tree<T>> },
}

impl<T> Tree<T> {
    fn leaf(item: T) -> Self {
        Tree::Leaf { item, active: true }
    }

    /// The leaves in order.
    pub fn items(&self) -> Vec<&T> {
        match self {
            Tree::Leaf { item, .. } => vec![item],
            Tree::Split { first, second, .. } => {
                let mut items = first.items();
                items.extend(second.items());
                items
            }
        }
    }

    /// The active leaf, else the first.
    pub fn active(&self) -> &T {
        fn find<T>(tree: &Tree<T>) -> Option<&T> {
            match tree {
                Tree::Leaf { item, active } => active.then_some(item),
                Tree::Split { first, second, .. } => find(first).or_else(|| find(second)),
            }
        }
        find(self).unwrap_or_else(|| self.items()[0])
    }

    fn has_active(&self) -> bool {
        match self {
            Tree::Leaf { active, .. } => *active,
            Tree::Split { first, second, .. } => first.has_active() || second.has_active(),
        }
    }

    fn activate_first(&mut self) {
        match self {
            Tree::Leaf { active, .. } => *active = true,
            Tree::Split { first, .. } => first.activate_first(),
        }
    }
}

/// A tab a sweep opens: one session, or several in their recorded split.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTab {
    /// The recorded manual title.
    pub title: Option<String>,
    pub zones: Tree<SessionInfo>,
}

impl LayoutTab {
    /// The session whose profile the tab takes: its active zone's.
    pub fn active(&self) -> &SessionInfo {
        self.zones.active()
    }

    /// The tab's one session, or the tab back when it is split.
    pub fn into_single(self) -> Result<SessionInfo, Self> {
        match self.zones {
            Tree::Leaf { item, .. } => Ok(item),
            zones => Err(Self { zones, ..self }),
        }
    }

    /// The split as a `ZoneTree` with ids from 1, each leaf's session, and the
    /// active zone's id.
    pub fn into_tree(self) -> (ZoneTree, Vec<(ZoneId, SessionInfo)>, ZoneId) {
        fn build(
            tree: Tree<SessionInfo>,
            next: &mut u64,
            leaves: &mut Vec<(ZoneId, SessionInfo)>,
            active_id: &mut Option<ZoneId>,
        ) -> ZoneTree {
            match tree {
                Tree::Leaf { item, active } => {
                    let id = ZoneId(*next);
                    *next += 1;
                    if active && active_id.is_none() {
                        *active_id = Some(id);
                    }
                    leaves.push((id, item));
                    ZoneTree::Leaf(id)
                }
                Tree::Split { axis, first, second } => ZoneTree::Split {
                    axis,
                    first: Box::new(build(*first, next, leaves, active_id)),
                    second: Box::new(build(*second, next, leaves, active_id)),
                },
            }
        }
        let (mut next, mut leaves, mut active) = (1, Vec::new(), None);
        let tree = build(self.zones, &mut next, &mut leaves, &mut active);
        (tree, leaves, active.unwrap_or(ZoneId(1)))
    }
}

/// `info`'s layout, when it is one this build reads and it names `info`
/// itself (a blob from another tab, or junk, places nothing).
fn layout_of(info: &SessionInfo) -> Option<Layout> {
    let layout: Layout = serde_json::from_value(info.layout.clone()?).ok()?;
    (layout.v == VERSION && !info.key.is_empty() && layout.zones.has_key(&info.key)).then_some(layout)
}

/// The tabs that `sessions` (those a sweep opens, detached and not taken by
/// a restored zone) open as. Sessions detached with one tab's layout are
/// that tab, its split tree pruned to the sessions here (a split's sibling
/// takes a missing zone's place; the first zone is active when the recorded
/// active one is missing). The rest are tabs of their own. A quit's tabs
/// keep their recorded order, together where the oldest session among them
/// sorts; a tab of its own sorts by its session; sessions sort by start time,
/// then id.
pub fn group(sessions: Vec<SessionInfo>) -> Vec<LayoutTab> {
    struct Group {
        /// `(quit, index)` for a recorded tab.
        tab: Option<(String, u32)>,
        layout: Option<Layout>,
        members: Vec<SessionInfo>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for info in sessions {
        let Some(layout) = layout_of(&info) else {
            groups.push(Group { tab: None, layout: None, members: vec![info] });
            continue;
        };
        let tab = (layout.quit.clone(), layout.index);
        match groups.iter_mut().find(|g| g.tab.as_ref() == Some(&tab)) {
            Some(group) => group.members.push(info),
            None => groups.push(Group { tab: Some(tab), layout: Some(layout), members: vec![info] }),
        }
    }

    // Each tab with its sort key: where its quit (or its own session) sorts,
    // then its index.
    let mut tabs: Vec<((u64, u32, String, u32), LayoutTab)> = Vec::new();
    let flat = |info: SessionInfo, tabs: &mut Vec<_>| {
        let key = (info.started, info.session, String::new(), 0);
        tabs.push((key, LayoutTab { title: None, zones: Tree::leaf(info) }));
    };
    for group in groups {
        let (Some((quit, index)), Some(layout)) = (group.tab, group.layout) else {
            group.members.into_iter().for_each(|info| flat(info, &mut tabs));
            continue;
        };
        let mut members: HashMap<String, SessionInfo> =
            group.members.into_iter().map(|info| (info.key.clone(), info)).collect();
        let Some(mut zones) = prune(layout.zones, &mut members) else { continue };
        if !zones.has_active() {
            zones.activate_first();
        }
        let oldest = zones.items().into_iter().map(|s| (s.started, s.session)).min().unwrap_or_default();
        let title = layout.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
        tabs.push(((oldest.0, oldest.1, quit, index), LayoutTab { title, zones }));
        // Members its tree doesn't name (another detach of the same tab
        // can't make these; kept anyway) are tabs of their own.
        let mut left: Vec<SessionInfo> = members.into_values().collect();
        left.sort_by_key(|s| (s.started, s.session));
        left.into_iter().for_each(|info| flat(info, &mut tabs));
    }
    // A quit sorts by its oldest session over all its tabs.
    let mut anchors: HashMap<String, (u64, u32)> = HashMap::new();
    for ((started, session, quit, _), _) in &tabs {
        if !quit.is_empty() {
            let anchor = anchors.entry(quit.clone()).or_insert((*started, *session));
            *anchor = (*anchor).min((*started, *session));
        }
    }
    tabs.sort_by_key(|((started, session, quit, index), _)| {
        let anchor = anchors.get(quit).copied().unwrap_or((*started, *session));
        (anchor, quit.clone(), *index)
    });
    tabs.into_iter().map(|(_, tab)| tab).collect()
}

/// `node` with each leaf's session taken from `members`, without the leaves
/// it has none for; `None` when none is left.
fn prune(node: Node, members: &mut HashMap<String, SessionInfo>) -> Option<Tree<SessionInfo>> {
    match node {
        Node::Leaf { key, active } => members.remove(&key).map(|item| Tree::Leaf { item, active }),
        Node::Split { split, first, second } => match (prune(*first, members), prune(*second, members)) {
            (Some(first), Some(second)) => Some(Tree::Split { axis: split, first: Box::new(first), second: Box::new(second) }),
            (one, other) => one.or(other),
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn split(axis: SplitAxis, first: ZoneTree, second: ZoneTree) -> ZoneTree {
        ZoneTree::Split { axis, first: Box::new(first), second: Box::new(second) }
    }

    fn key_of(id: ZoneId) -> Option<String> {
        // Zone 3 shows only a message.
        (id.0 != 3).then(|| format!("k{}", id.0))
    }

    #[test]
    fn the_blob_has_the_documented_shape() {
        let tree = split(SplitAxis::Horizontal, ZoneTree::Leaf(ZoneId(1)), ZoneTree::Leaf(ZoneId(2)));
        let recorded = record(&tree, ZoneId(2), &mut key_of, "q1", 4, Some(" logs ")).unwrap();
        assert_eq!(
            recorded,
            json!({
                "v": 1,
                "quit": "q1",
                "index": 4,
                "title": "logs",
                "zones": {
                    "split": "horizontal",
                    "first": {"key": "k1"},
                    "second": {"key": "k2", "active": true},
                },
            })
        );
        let single = record(&ZoneTree::Leaf(ZoneId(1)), ZoneId(1), &mut key_of, "q1", 0, Some("  ")).unwrap();
        assert_eq!(single, json!({"v": 1, "quit": "q1", "index": 0, "zones": {"key": "k1", "active": true}}));
    }

    #[test]
    fn recording_leaves_out_zones_without_a_session() {
        // ((1 | 3) / 2), active 3: zone 1 takes 3's place, and is active.
        let tree = split(
            SplitAxis::Vertical,
            split(SplitAxis::Horizontal, ZoneTree::Leaf(ZoneId(1)), ZoneTree::Leaf(ZoneId(3))),
            ZoneTree::Leaf(ZoneId(2)),
        );
        let recorded = record(&tree, ZoneId(3), &mut key_of, "q", 0, None).unwrap();
        assert_eq!(
            recorded["zones"],
            json!({"split": "vertical", "first": {"key": "k1", "active": true}, "second": {"key": "k2"}})
        );
        assert_eq!(record(&ZoneTree::Leaf(ZoneId(3)), ZoneId(3), &mut key_of, "q", 0, None), None);
        assert_eq!(quit_id().is_empty(), false);
        assert_ne!(quit_id(), quit_id(), "two quits differ");
    }

    fn info(session: u32, key: &str, started: u64) -> SessionInfo {
        SessionInfo { session, key: key.into(), started, ..Default::default() }
    }

    fn laid_out(session: u32, key: &str, started: u64, layout: serde_json::Value) -> SessionInfo {
        SessionInfo { layout: Some(layout), ..info(session, key, started) }
    }

    /// A tab of the keys `a` and `b` split, `b` active.
    fn pair(quit: &str, index: u32, a: &str, b: &str) -> serde_json::Value {
        json!({
            "v": 1, "quit": quit, "index": index, "title": "logs",
            "zones": {"split": "horizontal", "first": {"key": a}, "second": {"key": b, "active": true}},
        })
    }

    fn keys(tab: &LayoutTab) -> Vec<&str> {
        tab.zones.items().into_iter().map(|s| s.key.as_str()).collect()
    }

    #[test]
    fn a_recorded_split_comes_back_whole() {
        let tabs = group(vec![
            laid_out(1, "a", 10, pair("q", 0, "a", "b")),
            laid_out(2, "b", 11, pair("q", 0, "a", "b")),
        ]);
        assert_eq!(tabs.len(), 1);
        assert_eq!(keys(&tabs[0]), ["a", "b"]);
        assert_eq!(tabs[0].title.as_deref(), Some("logs"));
        assert_eq!(tabs[0].active().key, "b");
        let (tree, leaves, active) = tabs[0].clone().into_tree();
        assert_eq!(tree, split(SplitAxis::Horizontal, ZoneTree::Leaf(ZoneId(1)), ZoneTree::Leaf(ZoneId(2))));
        assert_eq!(leaves.iter().map(|(id, s)| (id.0, s.session)).collect::<Vec<_>>(), [(1, 1), (2, 2)]);
        assert_eq!(active, ZoneId(2));
        assert!(tabs[0].clone().into_single().is_err());
    }

    #[test]
    fn a_missing_zone_leaves_its_sibling_and_the_first_is_active() {
        // `b` ended, or was taken by a restored zone or another Brindle.
        let tabs = group(vec![laid_out(1, "a", 10, pair("q", 0, "a", "b"))]);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].title.as_deref(), Some("logs"));
        let single = tabs[0].clone().into_single().unwrap();
        assert_eq!(single.key, "a");
        assert_eq!(tabs[0].zones, Tree::Leaf { item: single, active: true });
    }

    #[test]
    fn unusable_layouts_open_flat() {
        let tabs = group(vec![
            // Names another session only.
            laid_out(1, "a", 10, pair("q", 0, "x", "y")),
            // Another format version.
            laid_out(2, "b", 20, json!({"v": 2, "quit": "q", "index": 1, "zones": {"key": "b"}})),
            // Not a layout.
            laid_out(3, "c", 30, json!("junk")),
            // A core that predates keys.
            laid_out(4, "", 40, json!({"v": 1, "quit": "q", "index": 2, "zones": {"key": ""}})),
            info(5, "e", 5),
        ]);
        let order: Vec<u32> = tabs.iter().map(|t| t.active().session).collect();
        assert_eq!(order, [5, 1, 2, 3, 4]);
        assert!(tabs.iter().all(|t| t.title.is_none() && t.zones.items().len() == 1));
    }

    #[test]
    fn a_quits_tabs_keep_their_order_where_its_oldest_session_sorts() {
        let tabs = group(vec![
            // Quit q: tab 1 (c alone) is older than tab 0 (a | b).
            laid_out(1, "a", 50, pair("q", 0, "a", "b")),
            laid_out(2, "b", 51, pair("q", 0, "a", "b")),
            laid_out(3, "c", 20, json!({"v": 1, "quit": "q", "index": 1, "zones": {"key": "c", "active": true}})),
            // Quit r: one tab, started between.
            laid_out(4, "d", 30, pair("r", 0, "d", "e")),
            laid_out(5, "e", 31, pair("r", 0, "d", "e")),
            // Flat sessions, before, between and after.
            info(6, "f", 10),
            info(7, "g", 25),
            info(8, "h", 60),
        ]);
        let order: Vec<Vec<&str>> = tabs.iter().map(keys).collect();
        assert_eq!(order, [vec!["f"], vec!["a", "b"], vec!["c"], vec!["g"], vec!["d", "e"], vec!["h"]]);
    }

    #[test]
    fn titles_are_trimmed_and_blank_ones_dropped() {
        let titled = |title: &str| json!({"v": 1, "quit": "q", "index": 0, "title": title, "zones": {"key": "a"}});
        assert_eq!(group(vec![laid_out(1, "a", 1, titled("  build "))])[0].title.as_deref(), Some("build"));
        assert_eq!(group(vec![laid_out(1, "a", 1, titled("   "))])[0].title, None);
    }

    #[test]
    fn a_recorded_layout_groups_back_into_its_tab() {
        let tree = split(
            SplitAxis::Vertical,
            ZoneTree::Leaf(ZoneId(1)),
            split(SplitAxis::Horizontal, ZoneTree::Leaf(ZoneId(2)), ZoneTree::Leaf(ZoneId(4))),
        );
        let layout = record(&tree, ZoneId(4), &mut key_of, "q", 0, None).unwrap();
        let tabs = group((1..=4).filter(|n| *n != 3).map(|n| laid_out(n, &format!("k{n}"), n.into(), layout.clone())).collect());
        assert_eq!(tabs.len(), 1);
        let (rebuilt, leaves, active) = tabs[0].clone().into_tree();
        assert_eq!(rebuilt, split(
            SplitAxis::Vertical,
            ZoneTree::Leaf(ZoneId(1)),
            split(SplitAxis::Horizontal, ZoneTree::Leaf(ZoneId(2)), ZoneTree::Leaf(ZoneId(3))),
        ));
        assert_eq!(leaves.iter().map(|(_, s)| s.session).collect::<Vec<_>>(), [1, 2, 4]);
        assert_eq!(active, ZoneId(3));
    }
}
