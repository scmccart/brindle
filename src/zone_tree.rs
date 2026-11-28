//! A local tab's binary split tree of zones and its normalized geometry,
//! and where a split's new zone runs: GPUI-free, so the agent's pure
//! modules name it without reaching a view (`zone_view.rs` re-exports it).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Profile;
use crate::remote::Host;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoneId(pub u64);

/// Which way `FocusZone*` moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitAxis {
    /// Left and right halves (`Split Zone Right`).
    Horizontal,
    /// Top and bottom halves (`Split Zone Down`).
    Vertical,
}

/// Normalized rectangle in `[0, 1] x [0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl UnitRect {
    pub const FULL: Self = Self { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };

    pub fn split(self, axis: SplitAxis) -> (Self, Self) {
        match axis {
            SplitAxis::Horizontal => {
                let half = self.w * 0.5;
                (
                    Self { x: self.x, y: self.y, w: half, h: self.h },
                    Self { x: self.x + half, y: self.y, w: half, h: self.h },
                )
            }
            SplitAxis::Vertical => {
                let half = self.h * 0.5;
                (
                    Self { x: self.x, y: self.y, w: self.w, h: half },
                    Self { x: self.x, y: self.y + half, w: self.w, h: half },
                )
            }
        }
    }

    fn center_x(self) -> f64 {
        self.x + self.w * 0.5
    }

    fn center_y(self) -> f64 {
        self.y + self.h * 0.5
    }
}

/// Binary split tree of zones in a local tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneTree {
    Leaf(ZoneId),
    Split {
        axis: SplitAxis,
        first: Box<ZoneTree>,
        second: Box<ZoneTree>,
    },
}

impl ZoneTree {
    pub fn new(id: ZoneId) -> Self {
        Self::Leaf(id)
    }

    /// Replaces `Leaf(target)` with a `Split` holding `target` in `first`
    /// and `new_id` in `second`. Returns whether `target` was found.
    pub fn split(&mut self, target: ZoneId, axis: SplitAxis, new_id: ZoneId) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    axis,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new_id)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, axis, new_id) || second.split(target, axis, new_id)
            }
        }
    }

    /// Removes `Leaf(target)`, promoting its sibling subtree in place of the
    /// parent `Split`. Returns `Some(promoted_leaves)` when removed from a
    /// split, or `None` if `target` was not found (or was the root leaf).
    pub fn remove(&mut self, target: ZoneId) -> Option<Vec<ZoneId>> {
        match self {
            Self::Leaf(_) => None,
            Self::Split { first, second, .. } => {
                if matches!(**first, Self::Leaf(id) if id == target) {
                    let promoted = second.leaves();
                    *self = (**second).clone();
                    return Some(promoted);
                }
                if matches!(**second, Self::Leaf(id) if id == target) {
                    let promoted = first.leaves();
                    *self = (**first).clone();
                    return Some(promoted);
                }
                first.remove(target).or_else(|| second.remove(target))
            }
        }
    }

    /// All leaf IDs in tree order (first before second).
    pub fn leaves(&self) -> Vec<ZoneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<ZoneId>) {
        match self {
            Self::Leaf(id) => out.push(*id),
            Self::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// Normalized `[0, 1] x [0, 1]` rectangles for every leaf in tree order.
    pub fn rects(&self) -> Vec<(ZoneId, UnitRect)> {
        let mut out = Vec::new();
        self.collect_rects(UnitRect::FULL, &mut out);
        out
    }

    fn collect_rects(&self, rect: UnitRect, out: &mut Vec<(ZoneId, UnitRect)>) {
        match self {
            Self::Leaf(id) => out.push((*id, rect)),
            Self::Split { axis, first, second } => {
                let (r1, r2) = rect.split(*axis);
                first.collect_rects(r1, out);
                second.collect_rects(r2, out);
            }
        }
    }

    /// Finds the leaf to focus when moving from `active` in `direction`:
    /// candidates must overlap on the perpendicular axis and lie in
    /// `direction`, ranked by nearest primary edge, then closest
    /// perpendicular center, then tree order.
    pub fn focus_target(&self, active: ZoneId, direction: Direction) -> Option<ZoneId> {
        const EPS: f64 = 1e-9;
        let rects = self.rects();
        let (_, cur) = rects.iter().copied().find(|(id, _)| *id == active)?;

        let mut best: Option<(ZoneId, f64, f64)> = None;
        for (id, cand) in rects {
            if id == active {
                continue;
            }
            let (primary_dist, perp_overlap, perp_dist) = match direction {
                Direction::Left => {
                    let dist = cur.x - (cand.x + cand.w);
                    let overlap = (cur.y + cur.h).min(cand.y + cand.h) - cur.y.max(cand.y);
                    (dist, overlap, (cur.center_y() - cand.center_y()).abs())
                }
                Direction::Right => {
                    let dist = cand.x - (cur.x + cur.w);
                    let overlap = (cur.y + cur.h).min(cand.y + cand.h) - cur.y.max(cand.y);
                    (dist, overlap, (cur.center_y() - cand.center_y()).abs())
                }
                Direction::Up => {
                    let dist = cur.y - (cand.y + cand.h);
                    let overlap = (cur.x + cur.w).min(cand.x + cand.w) - cur.x.max(cand.x);
                    (dist, overlap, (cur.center_x() - cand.center_x()).abs())
                }
                Direction::Down => {
                    let dist = cand.y - (cur.y + cur.h);
                    let overlap = (cur.x + cur.w).min(cand.x + cand.w) - cur.x.max(cand.x);
                    (dist, overlap, (cur.center_x() - cand.center_x()).abs())
                }
            };
            if primary_dist < -EPS || perp_overlap <= EPS {
                continue;
            }
            let replace = match best {
                None => true,
                Some((_, best_primary, best_perp)) => {
                    if (primary_dist - best_primary).abs() > EPS {
                        primary_dist < best_primary
                    } else if (perp_dist - best_perp).abs() > EPS {
                        perp_dist < best_perp
                    } else {
                        // Earlier in tree order wins ties.
                        false
                    }
                }
            };
            if replace {
                best = Some((id, primary_dist, perp_dist));
            }
        }
        best.map(|(id, _, _)| id)
    }
}

/// Where a zone of `profile` started by a split of a zone of `split`
/// running on `split_host` runs: on the split zone's endpoint when the
/// profile is its own (names match ignoring case, as profile lookups do),
/// even when the profile's target has moved since; any other profile
/// where it says.
pub fn split_host(split: &Profile, split_host: &Host, profile: &Profile) -> Host {
    if profile.name.eq_ignore_ascii_case(&split.name) { split_host.clone() } else { Host::of(profile) }
}

/// The directory a split's new zone starts in: the split zone's, when both
/// run on one endpoint (`same_host`), whatever the new profile's own
/// directory; else none, and the new zone's start takes its profile's
/// directory, else the home directory there. A directory never crosses
/// endpoints.
pub fn split_cwd(split_dir: Option<PathBuf>, same_host: bool) -> Option<PathBuf> {
    split_dir.filter(|_| same_host)
}

/// Where a new zone of `profile` split from a zone of `split` on
/// `split_host` in `split_dir` runs ([`split_host`]) and the directory it
/// asks to start in ([`split_cwd`]): the split's rule, which the
/// conversations palette's directory follows too
/// (`crate::agent::conversations::list_dir`).
pub fn split_target(split: &Profile, split_host_now: &Host, split_dir: Option<PathBuf>, profile: &Profile) -> (Host, Option<PathBuf>) {
    let host = split_host(split, split_host_now, profile);
    let same_host = *split_host_now == host;
    (host, split_cwd(split_dir, same_host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// A split of the zone's own profile stays on its endpoint after the
    /// profile's target moved; another profile runs where it says; a
    /// directory never crosses endpoints.
    #[test]
    fn a_split_runs_where_its_profile_says_and_keeps_a_directory_on_its_host() {
        let ssh = |host: &str| Config::parse(&format!(
            "[[profiles]]\nname = \"Remote\"\n[profiles.remote]\ntype = \"ssh\"\nhost = \"{host}\"\n"
        ))
        .expect("the test config")
        .config
        .profiles
        .remove(0);
        let (old, moved) = (ssh("old-host"), ssh("new-host"));
        let local = Profile { name: "Shell".into(), ..Profile::default() };
        let on_old = Host::of(&old);
        assert_eq!(split_host(&old, &on_old, &moved), on_old, "its own profile stays where the zone runs");
        assert_eq!(split_host(&old, &on_old, &local), Host::Local, "another profile runs where it says");
        assert_eq!(split_host(&local, &Host::Local, &moved), Host::of(&moved));
        let here = Some(PathBuf::from("/srv/app"));
        assert_eq!(split_cwd(here.clone(), true), here, "the same host: the split zone's directory");
        assert_eq!(split_cwd(here.clone(), Host::Local == on_old), None, "remote to local");
        assert_eq!(split_cwd(here.clone(), on_old == Host::of(&moved)), None, "between two remotes");
        assert_eq!(split_cwd(None, true), None);
    }

    #[test]
    fn tree_split_remove_and_rects() {
        let z1 = ZoneId(1);
        let z2 = ZoneId(2);
        let z3 = ZoneId(3);
        let mut tree = ZoneTree::new(z1);
        assert_eq!(tree.leaves(), vec![z1]);
        assert_eq!(tree.rects(), vec![(z1, UnitRect::FULL)]);

        // Split z1 right -> z1 (left) and z2 (right).
        assert!(tree.split(z1, SplitAxis::Horizontal, z2));
        assert_eq!(
            tree.rects(),
            vec![
                (z1, UnitRect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }),
                (z2, UnitRect { x: 0.5, y: 0.0, w: 0.5, h: 1.0 }),
            ]
        );

        // Split z2 down -> z2 (top-right) and z3 (bottom-right).
        assert!(tree.split(z2, SplitAxis::Vertical, z3));
        assert_eq!(
            tree.rects(),
            vec![
                (z1, UnitRect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }),
                (z2, UnitRect { x: 0.5, y: 0.0, w: 0.5, h: 0.5 }),
                (z3, UnitRect { x: 0.5, y: 0.5, w: 0.5, h: 0.5 }),
            ]
        );

        // Removing z1 promotes the right subtree (z2 and z3) to fill the tab.
        let promoted = tree.remove(z1).unwrap();
        assert_eq!(promoted, vec![z2, z3]);
        assert_eq!(
            tree.rects(),
            vec![
                (z2, UnitRect { x: 0.0, y: 0.0, w: 1.0, h: 0.5 }),
                (z3, UnitRect { x: 0.0, y: 0.5, w: 1.0, h: 0.5 }),
            ]
        );

        // Removing z2 leaves z3 as the single root leaf.
        let promoted = tree.remove(z2).unwrap();
        assert_eq!(promoted, vec![z3]);
        assert_eq!(tree, ZoneTree::Leaf(z3));
        assert_eq!(tree.remove(z3), None);
    }

    #[test]
    fn directional_focus_navigation() {
        let z1 = ZoneId(1);
        let z2 = ZoneId(2);
        let z3 = ZoneId(3);
        let mut tree = ZoneTree::new(z1);
        tree.split(z1, SplitAxis::Horizontal, z2);

        // Side-by-side moves and outer-edge no-ops.
        assert_eq!(tree.focus_target(z1, Direction::Right), Some(z2));
        assert_eq!(tree.focus_target(z2, Direction::Left), Some(z1));
        assert_eq!(tree.focus_target(z1, Direction::Left), None);
        assert_eq!(tree.focus_target(z2, Direction::Right), None);
        assert_eq!(tree.focus_target(z1, Direction::Up), None);
        assert_eq!(tree.focus_target(z1, Direction::Down), None);

        // Split right half into z2 (top-right) and z3 (bottom-right).
        // Moving right from tall left z1 breaks center-distance tie by tree order -> z2.
        tree.split(z2, SplitAxis::Vertical, z3);
        assert_eq!(tree.focus_target(z1, Direction::Right), Some(z2));
        assert_eq!(tree.focus_target(z2, Direction::Down), Some(z3));
        assert_eq!(tree.focus_target(z3, Direction::Up), Some(z2));
        assert_eq!(tree.focus_target(z2, Direction::Left), Some(z1));
        assert_eq!(tree.focus_target(z3, Direction::Left), Some(z1));

        // When left half is split into top-quarter and bottom-three-quarters
        // (via two vertical splits on left), perpendicular center picks the closer one.
        let z4 = ZoneId(4);
        let z5 = ZoneId(5);
        tree.split(z1, SplitAxis::Vertical, z4); // z1 top-left, z4 bottom-left
        tree.split(z4, SplitAxis::Vertical, z5); // z4 mid-left (0.5..0.75), z5 bottom-left (0.75..1.0)
        // From z5 (center_y = 0.875), moving right goes to z3 (center_y = 0.75) rather than z2 (0.25).
        assert_eq!(tree.focus_target(z5, Direction::Right), Some(z3));
        // From z1 (center_y = 0.25), moving right goes to z2 (center_y = 0.25).
        assert_eq!(tree.focus_target(z1, Direction::Right), Some(z2));
    }
}
