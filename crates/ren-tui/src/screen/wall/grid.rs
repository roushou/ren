//! The panes a terminal has room for, and which session each one shows. A
//! session keeps its pane while it stays on the wall, so panes never trade
//! places as activity moves; a pane freed by a session that went quiet
//! takes the next session that needs one.

use crate::Size;
use ren_recorder::id::SessionId;

/// Panes in columns and rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub columns: u16,
    pub rows: u16,
}

impl Grid {
    /// The narrowest a pane gets, in cells.
    pub const PANE_WIDTH: u16 = 56;
    /// The shortest a pane gets, in lines.
    pub const PANE_HEIGHT: u16 = 10;
    /// Lines below the panes: the status line.
    pub const STATUS_LINES: u16 = 1;
    /// At most this many panes across and down.
    pub const MAX: u16 = 4;

    /// As many panes as fit, at least one.
    pub fn fit(size: Size) -> Self {
        let height = size.height.saturating_sub(Self::STATUS_LINES);
        Self {
            columns: (size.width / Self::PANE_WIDTH).clamp(1, Self::MAX),
            rows: (height / Self::PANE_HEIGHT).clamp(1, Self::MAX),
        }
    }

    pub fn panes(self) -> usize {
        usize::from(self.columns) * usize::from(self.rows)
    }

    /// The grid for `sessions` within what `size` has room for: as few
    /// panes left empty as can be, then panes whose shape is closest to a
    /// terminal pane's usual one, about 3.5 cells across for each line
    /// down. One session fills the screen.
    pub fn for_sessions(size: Size, sessions: usize) -> Self {
        let room = Self::fit(size);
        let wanted = sessions.clamp(1, room.panes());
        let height = f64::from(size.height.saturating_sub(Self::STATUS_LINES).max(1));
        let width = f64::from(size.width.max(1));
        let mut best: Option<(f64, Self)> = None;
        for columns in 1..=room.columns {
            for rows in 1..=room.rows {
                let grid = Self { columns, rows };
                if grid.panes() < wanted {
                    continue;
                }
                let empty = u32::try_from(grid.panes() - wanted).unwrap_or(u32::MAX);
                let shape = (width / f64::from(columns)) / (height / f64::from(rows));
                let score = f64::from(empty) + (shape / 3.5).ln().abs();
                if best.is_none_or(|(least, _)| score < least) {
                    best = Some((score, grid));
                }
            }
        }
        best.map_or(room, |(_, grid)| grid)
    }
}

/// Which session each pane shows, in pane order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Placement {
    panes: Vec<Option<SessionId>>,
}

impl Placement {
    /// Fits the placement to `grid`, keeps sessions still in `wanted`
    /// where they are, and fills free panes from `wanted` in its order.
    /// Returns how many wanted sessions have no pane.
    pub(crate) fn arrange(&mut self, grid: Grid, wanted: &[SessionId]) -> usize {
        self.panes.resize(grid.panes(), None);
        for pane in &mut self.panes {
            if pane.is_some_and(|session| !wanted.contains(&session)) {
                *pane = None;
            }
        }
        // A shrinking grid drops its last panes: their sessions move to a
        // free pane, if any, in the order they are wanted.
        let waiting: Vec<SessionId> = wanted
            .iter()
            .filter(|session| !self.panes.contains(&Some(**session)))
            .copied()
            .collect();
        let mut waiting = waiting.into_iter();
        for pane in self.panes.iter_mut().filter(|pane| pane.is_none()) {
            *pane = waiting.next();
        }
        waiting.count()
    }

    pub(crate) fn panes(&self) -> &[Option<SessionId>] {
        &self.panes
    }
}

#[cfg(test)]
mod tests {
    use super::{Grid, Placement};
    use crate::Size;
    use ren_recorder::id::SessionId;
    use uuid::Uuid;

    fn session(n: u128) -> SessionId {
        SessionId::restore(Uuid::from_u128(n))
    }

    #[test]
    fn a_terminal_fits_as_many_panes_as_have_room() {
        let fit = |width, height| {
            let grid = Grid::fit(Size { width, height });
            (grid.columns, grid.rows)
        };
        assert_eq!(fit(80, 24), (1, 2));
        assert_eq!(fit(200, 50), (3, 4));
        assert_eq!(fit(20, 5), (1, 1));
        assert_eq!(fit(400, 200), (4, 4));
    }

    #[test]
    fn sessions_keep_their_panes_and_free_ones_are_refilled() {
        let grid = Grid {
            columns: 2,
            rows: 1,
        };
        let mut placement = Placement::default();
        assert_eq!(
            placement.arrange(grid, &[session(1), session(2), session(3)]),
            1
        );
        assert_eq!(placement.panes(), [Some(session(1)), Some(session(2))]);
        // Session 1 went quiet: its pane takes the next, and 2 stays put
        // although it is now wanted first.
        assert_eq!(placement.arrange(grid, &[session(2), session(3)]), 0);
        assert_eq!(placement.panes(), [Some(session(3)), Some(session(2))]);
        assert_eq!(placement.arrange(grid, &[]), 0);
        assert_eq!(placement.panes(), [None, None]);
    }

    #[test]
    fn a_smaller_grid_moves_dropped_sessions_to_free_panes() {
        let wide = Grid {
            columns: 3,
            rows: 1,
        };
        let mut placement = Placement::default();
        placement.arrange(wide, &[session(1), session(2), session(3)]);
        placement.arrange(wide, &[session(2), session(3)]);
        assert_eq!(
            placement.panes(),
            [None, Some(session(2)), Some(session(3))]
        );
        let narrow = Grid {
            columns: 2,
            rows: 1,
        };
        assert_eq!(placement.arrange(narrow, &[session(2), session(3)]), 0);
        assert_eq!(placement.panes(), [Some(session(3)), Some(session(2))]);
    }
}
