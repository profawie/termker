//! Terminals shown together in one window. A view is a tree: each leaf is
//! one session's pane, each branch splits its space between two smaller
//! trees, side by side or one above the other. Sessions are put together by
//! dragging one onto an edge of another.

use eframe::egui::{Pos2, Rect, Vec2, pos2};
use uuid::Uuid;

/// The edge of a pane a session is dropped on: the session takes that half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    /// The edge of `rect` closest to `pos`, measured as a share of the
    /// pane's width or height so that wide and tall panes split alike.
    pub fn nearest(rect: Rect, pos: Pos2) -> Side {
        let size = rect.size().max(Vec2::splat(1.0));
        let x = ((pos.x - rect.left()) / size.x).clamp(0.0, 1.0);
        let y = ((pos.y - rect.top()) / size.y).clamp(0.0, 1.0);
        let options = [
            (x, Side::Left),
            (1.0 - x, Side::Right),
            (y, Side::Top),
            (1.0 - y, Side::Bottom),
        ];
        options
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, side)| side)
            .unwrap_or(Side::Right)
    }

    /// The half of `rect` that a session dropped on this side will take.
    pub fn half(self, rect: Rect) -> Rect {
        let center = rect.center();
        match self {
            Side::Left => Rect::from_min_max(rect.min, pos2(center.x, rect.bottom())),
            Side::Right => Rect::from_min_max(pos2(center.x, rect.top()), rect.max),
            Side::Top => Rect::from_min_max(rect.min, pos2(rect.right(), center.y)),
            Side::Bottom => Rect::from_min_max(pos2(rect.left(), center.y), rect.max),
        }
    }

    /// Words for the hint under the dragged session.
    pub fn hint(self) -> &'static str {
        match self {
            Side::Left => "Drop to show it on the left",
            Side::Right => "Drop to show it on the right",
            Side::Top => "Drop to show it above",
            Side::Bottom => "Drop to show it below",
        }
    }

    fn axis(self) -> Axis {
        match self {
            Side::Left | Side::Right => Axis::SideBySide,
            Side::Top | Side::Bottom => Axis::Stacked,
        }
    }

    fn goes_first(self) -> bool {
        matches!(self, Side::Left | Side::Top)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Left and right.
    SideBySide,
    /// Top and bottom.
    Stacked,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tile {
    /// One session's terminal.
    Pane(Uuid),
    Split(Box<Split>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    pub axis: Axis,
    /// Share of the space the first tile gets.
    pub ratio: f32,
    /// Left or top.
    pub first: Tile,
    /// Right or bottom.
    pub second: Tile,
}

/// The line between the two halves of a split, which can be dragged to
/// give one side more room.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    /// Which way to go from the top of the tree to reach this split:
    /// `false` for the first tile, `true` for the second.
    pub path: Vec<bool>,
    pub axis: Axis,
    /// The gap between the two halves, where the pointer grabs it.
    pub rect: Rect,
    /// All the space the split shares out.
    pub span: Rect,
}

/// The smallest share a pane can be given by dragging a divider.
pub const MIN_RATIO: f32 = 0.15;

impl Tile {
    /// Session ids of every pane, left to right and top to bottom.
    pub fn panes(&self) -> Vec<Uuid> {
        let mut panes = Vec::new();
        self.collect(&mut panes);
        panes
    }

    fn collect(&self, panes: &mut Vec<Uuid>) {
        match self {
            Tile::Pane(id) => panes.push(*id),
            Tile::Split(split) => {
                split.first.collect(panes);
                split.second.collect(panes);
            }
        }
    }

    pub fn contains(&self, id: Uuid) -> bool {
        match self {
            Tile::Pane(pane) => *pane == id,
            Tile::Split(split) => split.first.contains(id) || split.second.contains(id),
        }
    }

    pub fn is_split(&self) -> bool {
        matches!(self, Tile::Split(_))
    }

    /// The tree without the pane of `id`; its neighbour takes the space.
    /// `None` when nothing is left.
    pub fn without(self, id: Uuid) -> Option<Tile> {
        match self {
            Tile::Pane(pane) if pane == id => None,
            Tile::Pane(_) => Some(self),
            Tile::Split(split) => {
                let Split {
                    axis,
                    ratio,
                    first,
                    second,
                } = *split;
                match (first.without(id), second.without(id)) {
                    (Some(first), Some(second)) => Some(Tile::Split(Box::new(Split {
                        axis,
                        ratio,
                        first,
                        second,
                    }))),
                    (Some(only), None) | (None, Some(only)) => Some(only),
                    (None, None) => None,
                }
            }
        }
    }

    /// Put `new` in the pane of `old`, keeping its place.
    pub fn replace(&mut self, old: Uuid, new: Uuid) {
        match self {
            Tile::Pane(pane) => {
                if *pane == old {
                    *pane = new;
                }
            }
            Tile::Split(split) => {
                split.first.replace(old, new);
                split.second.replace(old, new);
            }
        }
    }

    /// Split the pane of `target` in two and show `new` on its `side`.
    /// Returns `false` when `target` is not in this tree.
    pub fn insert(&mut self, target: Uuid, new: Uuid, side: Side) -> bool {
        match self {
            Tile::Pane(pane) if *pane == target => {
                let (first, second) = if side.goes_first() {
                    (Tile::Pane(new), Tile::Pane(target))
                } else {
                    (Tile::Pane(target), Tile::Pane(new))
                };
                *self = Tile::Split(Box::new(Split {
                    axis: side.axis(),
                    ratio: 0.5,
                    first,
                    second,
                }));
                true
            }
            Tile::Pane(_) => false,
            Tile::Split(split) => {
                split.first.insert(target, new, side) || split.second.insert(target, new, side)
            }
        }
    }

    /// Where each pane goes in `rect`, with `gap` between neighbours, and
    /// where the dividers are.
    pub fn arrange(&self, rect: Rect, gap: f32) -> (Vec<(Uuid, Rect)>, Vec<Divider>) {
        let mut panes = Vec::new();
        let mut dividers = Vec::new();
        self.arrange_into(rect, gap, &mut Vec::new(), &mut panes, &mut dividers);
        (panes, dividers)
    }

    fn arrange_into(
        &self,
        rect: Rect,
        gap: f32,
        path: &mut Vec<bool>,
        panes: &mut Vec<(Uuid, Rect)>,
        dividers: &mut Vec<Divider>,
    ) {
        match self {
            Tile::Pane(id) => panes.push((*id, rect)),
            Tile::Split(split) => {
                let ratio = split.ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
                let half = gap / 2.0;
                let (first, divider, second) = match split.axis {
                    Axis::SideBySide => {
                        let at = rect.left() + rect.width() * ratio;
                        (
                            Rect::from_min_max(rect.min, pos2(at - half, rect.bottom())),
                            Rect::from_min_max(
                                pos2(at - half, rect.top()),
                                pos2(at + half, rect.bottom()),
                            ),
                            Rect::from_min_max(pos2(at + half, rect.top()), rect.max),
                        )
                    }
                    Axis::Stacked => {
                        let at = rect.top() + rect.height() * ratio;
                        (
                            Rect::from_min_max(rect.min, pos2(rect.right(), at - half)),
                            Rect::from_min_max(
                                pos2(rect.left(), at - half),
                                pos2(rect.right(), at + half),
                            ),
                            Rect::from_min_max(pos2(rect.left(), at + half), rect.max),
                        )
                    }
                };
                dividers.push(Divider {
                    path: path.clone(),
                    axis: split.axis,
                    rect: divider,
                    span: rect,
                });
                path.push(false);
                split.first.arrange_into(first, gap, path, panes, dividers);
                path.pop();
                path.push(true);
                split
                    .second
                    .arrange_into(second, gap, path, panes, dividers);
                path.pop();
            }
        }
    }

    /// The split reached by following `path` from the top.
    pub fn split_at_mut(&mut self, path: &[bool]) -> Option<&mut Split> {
        let Tile::Split(split) = self else {
            return None;
        };
        match path.split_first() {
            None => Some(split),
            Some((false, rest)) => split.first.split_at_mut(rest),
            Some((true, rest)) => split.second.split_at_mut(rest),
        }
    }
}

impl Divider {
    /// The share of the split's space left of (or above) `pointer`.
    pub fn ratio_at(&self, pointer: Pos2) -> f32 {
        let ratio = match self.axis {
            Axis::SideBySide => (pointer.x - self.span.left()) / self.span.width().max(1.0),
            Axis::Stacked => (pointer.y - self.span.top()) / self.span.height().max(1.0),
        };
        ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO)
    }
}

/// Views with `moved` taken out of whichever one held it. Views left
/// empty are dropped.
pub fn take_out(views: Vec<Tile>, moved: Uuid) -> Vec<Tile> {
    views
        .into_iter()
        .filter_map(|view| view.without(moved))
        .collect()
}

/// Keep `views` in step with the open sessions: a session that is in no
/// view gets one of its own, and panes of sessions that are gone are
/// removed.
pub fn sync(views: Vec<Tile>, sessions: &[Uuid]) -> Vec<Tile> {
    let mut views: Vec<Tile> = views
        .into_iter()
        .filter_map(|view| {
            view.panes()
                .into_iter()
                .filter(|id| !sessions.contains(id))
                .try_fold(view, |view, gone| view.without(gone).ok_or(()))
                .ok()
        })
        .collect();
    // A session shown twice keeps its first place only.
    let mut seen = Vec::new();
    views = views
        .into_iter()
        .filter_map(|view| {
            let twice: Vec<Uuid> = view
                .panes()
                .into_iter()
                .filter(|id| {
                    let again = seen.contains(id);
                    seen.push(*id);
                    again
                })
                .collect();
            twice
                .into_iter()
                .try_fold(view, |view, id| view.without(id).ok_or(()))
                .ok()
        })
        .collect();
    for id in sessions {
        if !views.iter().any(|view| view.contains(*id)) {
            views.push(Tile::Pane(*id));
        }
    }
    views
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<Uuid> {
        (0..n).map(|_| Uuid::new_v4()).collect()
    }

    fn area() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))
    }

    #[test]
    fn nearest_edge() {
        let rect = area();
        assert_eq!(Side::nearest(rect, pos2(10.0, 300.0)), Side::Left);
        assert_eq!(Side::nearest(rect, pos2(790.0, 300.0)), Side::Right);
        assert_eq!(Side::nearest(rect, pos2(400.0, 10.0)), Side::Top);
        assert_eq!(Side::nearest(rect, pos2(400.0, 590.0)), Side::Bottom);
        // Outside the pane still gives the closest edge.
        assert_eq!(Side::nearest(rect, pos2(-50.0, 300.0)), Side::Left);
    }

    #[test]
    fn half_is_on_the_side() {
        let rect = area();
        assert_eq!(Side::Left.half(rect).max.x, 400.0);
        assert_eq!(Side::Right.half(rect).min.x, 400.0);
        assert_eq!(Side::Top.half(rect).max.y, 300.0);
        assert_eq!(Side::Bottom.half(rect).min.y, 300.0);
    }

    #[test]
    fn insert_puts_new_pane_on_the_side() {
        let id = ids(2);
        let mut tile = Tile::Pane(id[0]);
        assert!(tile.insert(id[0], id[1], Side::Left));
        assert_eq!(tile.panes(), vec![id[1], id[0]]);
        let Tile::Split(split) = &tile else {
            panic!("split expected");
        };
        assert_eq!(split.axis, Axis::SideBySide);

        let mut tile = Tile::Pane(id[0]);
        assert!(tile.insert(id[0], id[1], Side::Bottom));
        assert_eq!(tile.panes(), vec![id[0], id[1]]);
        let Tile::Split(split) = &tile else {
            panic!("split expected");
        };
        assert_eq!(split.axis, Axis::Stacked);
    }

    #[test]
    fn insert_into_nested_pane() {
        let id = ids(3);
        let mut tile = Tile::Pane(id[0]);
        tile.insert(id[0], id[1], Side::Right);
        assert!(tile.insert(id[1], id[2], Side::Bottom));
        assert_eq!(tile.panes(), id);
        assert!(!tile.insert(Uuid::new_v4(), id[2], Side::Top));
    }

    #[test]
    fn without_collapses_the_split() {
        let id = ids(3);
        let mut tile = Tile::Pane(id[0]);
        tile.insert(id[0], id[1], Side::Right);
        tile.insert(id[1], id[2], Side::Bottom);
        let tile = tile.without(id[1]).expect("two panes left");
        assert_eq!(tile.panes(), vec![id[0], id[2]]);
        let tile = tile.without(id[0]).expect("one pane left");
        assert_eq!(tile, Tile::Pane(id[2]));
        assert_eq!(tile.without(id[2]), None);
    }

    #[test]
    fn replace_keeps_the_place() {
        let id = ids(3);
        let mut tile = Tile::Pane(id[0]);
        tile.insert(id[0], id[1], Side::Right);
        tile.replace(id[0], id[2]);
        assert_eq!(tile.panes(), vec![id[2], id[1]]);
    }

    #[test]
    fn arrange_shares_out_the_space() {
        let id = ids(3);
        let mut tile = Tile::Pane(id[0]);
        tile.insert(id[0], id[1], Side::Right);
        tile.insert(id[1], id[2], Side::Bottom);
        let (panes, dividers) = tile.arrange(area(), 4.0);
        assert_eq!(panes.len(), 3);
        assert_eq!(
            panes[0],
            (
                id[0],
                Rect::from_min_max(pos2(0.0, 0.0), pos2(398.0, 600.0))
            )
        );
        assert_eq!(
            panes[1].1,
            Rect::from_min_max(pos2(402.0, 0.0), pos2(800.0, 298.0))
        );
        assert_eq!(
            panes[2].1,
            Rect::from_min_max(pos2(402.0, 302.0), pos2(800.0, 600.0))
        );
        assert_eq!(dividers.len(), 2);
        assert_eq!(dividers[0].path, Vec::<bool>::new());
        assert_eq!(dividers[1].path, vec![true]);
        assert_eq!(dividers[1].axis, Axis::Stacked);
    }

    #[test]
    fn divider_drag_changes_ratio() {
        let id = ids(3);
        let mut tile = Tile::Pane(id[0]);
        tile.insert(id[0], id[1], Side::Right);
        tile.insert(id[1], id[2], Side::Bottom);
        let (_, dividers) = tile.arrange(area(), 4.0);
        let lower = &dividers[1];
        let ratio = lower.ratio_at(pos2(600.0, 450.0));
        assert!((ratio - 0.75).abs() < 1e-6);
        tile.split_at_mut(&lower.path).expect("split").ratio = ratio;
        let (panes, _) = tile.arrange(area(), 4.0);
        assert_eq!(panes[1].1.max.y, 448.0);
        // Never smaller than the minimum.
        assert_eq!(dividers[0].ratio_at(pos2(5.0, 0.0)), MIN_RATIO);
    }

    #[test]
    fn take_out_drops_empty_views() {
        let id = ids(3);
        let mut first = Tile::Pane(id[0]);
        first.insert(id[0], id[1], Side::Right);
        let views = take_out(vec![first, Tile::Pane(id[2])], id[2]);
        assert_eq!(views.len(), 1);
        let views = take_out(views, id[0]);
        assert_eq!(views, vec![Tile::Pane(id[1])]);
    }

    #[test]
    fn sync_follows_the_sessions() {
        let id = ids(4);
        let mut first = Tile::Pane(id[0]);
        first.insert(id[0], id[1], Side::Right);
        // id[1] closed, id[2] shown twice, id[3] new.
        let views = vec![first, Tile::Pane(id[2]), Tile::Pane(id[2])];
        let views = sync(views, &[id[0], id[2], id[3]]);
        assert_eq!(
            views,
            vec![Tile::Pane(id[0]), Tile::Pane(id[2]), Tile::Pane(id[3])]
        );
    }
}
