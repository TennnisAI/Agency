//! Where a dragged pin lands (AGE-161): the rank writes for dropping one pinned
//! run onto another, planned against every pin in the project.
//!
//! Pure, so the caller can read the pins and write the plan under one registry
//! lock. It used to be planned in the UI and sent one rank at a time, which
//! went wrong two ways. The UI only lists live runs, so a midpoint could land
//! on an archived pin's rank and tie with it when the run was restored. A
//! renumber was n separate writes from a board up to a tick old, and one
//! refused write left the rest half applied, in an order nobody chose.

/// Ranks closer than this are treated as collapsed, and every pin is
/// renumbered. The same threshold as the issue board's `issueRank.ts`.
const MIN_GAP: f64 = 1e-6;

/// The `(id, rank)` writes for dropping pinned run `id` onto pinned run
/// `over`. The dragged run takes the other's place: before it when dragged up
/// the board, after it when dragged down.
///
/// `pins` is every pinned run in the project, archived ones included, in board
/// order. The board a user drags on is those pins with the archived ones left
/// out, so moving before or after `over` here is the same move there, and an
/// archived pin keeps the place it was pinned to for when it is restored.
///
/// One write when a midpoint fits; every pin renumbered from 1 when the gap
/// has collapsed, or two ranks already tie. Empty for a drop on its own
/// place. `Err` names a run that is not a pin in this project: a drop never
/// pins anything.
pub fn plan_move(
    pins: &[(String, f64)],
    id: &str,
    over: &str,
) -> Result<Vec<(String, f64)>, String> {
    let find = |x: &str| {
        pins.iter().position(|(p, _)| p == x).ok_or_else(|| format!("run is not pinned: {x}"))
    };
    let from = find(id)?;
    let to = find(over)?;
    if from == to {
        return Ok(Vec::new());
    }
    let mut next = pins.to_vec();
    let moved = next.remove(from);
    next.insert(to, moved);

    let before = to.checked_sub(1).map(|i| next[i].1);
    let after = next.get(to + 1).map(|p| p.1);
    let mid = match (before, after) {
        (Some(b), Some(a)) => (b + a) / 2.0,
        (Some(b), None) => b + 1.0,
        (None, Some(a)) => a - 1.0,
        (None, None) => unreachable!("two distinct pins, so the moved one has a neighbour"),
    };
    let collapsed =
        before.is_some_and(|b| mid - b < MIN_GAP) || after.is_some_and(|a| a - mid < MIN_GAP);
    if collapsed {
        return Ok(next.into_iter().enumerate().map(|(i, (p, _))| (p, (i + 1) as f64)).collect());
    }
    Ok(vec![(next[to].0.clone(), mid)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pins(list: &[(&str, f64)]) -> Vec<(String, f64)> {
        list.iter().map(|(id, r)| (id.to_string(), *r)).collect()
    }

    fn plan(list: &[(&str, f64)], id: &str, over: &str) -> Vec<(String, f64)> {
        plan_move(&pins(list), id, over).unwrap()
    }

    #[test]
    fn lands_between_its_new_neighbours_with_one_write() {
        let board = [("c", 1.0), ("b", 2.0), ("e", 3.0)];
        assert_eq!(plan(&board, "e", "b"), pins(&[("e", 1.5)]), "dragged up: before b");
        assert_eq!(plan(&board, "c", "b"), pins(&[("c", 2.5)]), "dragged down: after b");
    }

    #[test]
    fn goes_ahead_of_the_first_pin_and_behind_the_last() {
        let board = [("c", 1.0), ("b", 2.0), ("e", 3.0)];
        assert_eq!(plan(&board, "b", "c"), pins(&[("b", 0.0)]));
        assert_eq!(plan(&board, "c", "e"), pins(&[("c", 4.0)]));
    }

    #[test]
    fn a_drop_on_its_own_place_writes_nothing() {
        assert!(plan(&[("a", 1.0), ("b", 2.0)], "a", "a").is_empty());
    }

    // Archived B holds rank 2 for when it is restored. The board shows A, C, D;
    // D dropped on C used to take the midpoint of A and C, which is 2.
    #[test]
    fn an_archived_pin_keeps_its_rank() {
        let all = [("a", 1.0), ("b", 2.0), ("c", 3.0), ("d", 4.0)];
        assert_eq!(plan(&all, "d", "c"), pins(&[("d", 2.5)]));
    }

    #[test]
    fn renumbers_every_pin_once_the_gap_has_collapsed() {
        let tight = [("x", 1.0), ("y", 1.0 + 1e-7), ("z", 2.0)];
        assert_eq!(plan(&tight, "z", "y"), pins(&[("x", 1.0), ("z", 2.0), ("y", 3.0)]));
    }

    // A tie leaves nothing to take the midpoint of, so the plan resolves it
    // rather than writing a third equal rank.
    #[test]
    fn a_tie_is_renumbered_away() {
        let tied = [("x", 1.0), ("y", 2.0), ("z", 2.0)];
        assert_eq!(plan(&tied, "x", "y"), pins(&[("y", 1.0), ("x", 2.0), ("z", 3.0)]));
    }

    // Unpinned from the sidebar while the pointer was down, at either end.
    #[test]
    fn refuses_a_run_that_is_not_a_pin() {
        let board = pins(&[("a", 1.0), ("b", 2.0)]);
        assert!(plan_move(&board, "gone", "a").is_err());
        assert!(plan_move(&board, "a", "gone").is_err());
    }
}
