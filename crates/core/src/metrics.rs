// crates/core/src/metrics.rs
//! Observable counters reported by the headless trace.
//!
//! `SPEC-RT-047` requires query-contract acceptance to be counted, not judged by
//! type names or boolean returns. P1 only *reports* the counters; the numeric
//! pass criteria belong to `SPEC-GATE-A0-07` / `-08` and are not claimed.

use std::ops::AddAssign;

/// One snapshot of the monotonic counters a headless run exposes.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Counters {
    /// `query` cache evaluations (one per `(store_epoch, filter)` miss).
    pub query_evals: u64,
    /// `RowSet` materializations (stable-id collections).
    pub id_materializations: u64,
    /// Row-set constructions from a store scan (`SPEC-GATE-A0-07`). A cache
    /// hit costs nothing; an empty result costs nothing either, because it is
    /// answered from the membership counters.
    pub cache_builds: u64,
    /// Rows scanned while building a row set (`SPEC-GATE-A0-08`). This is the
    /// counter that must stay at zero for an empty result: a `collect()`-based
    /// `is_empty` would move it.
    pub row_visits: u64,
    /// Retained views mounted.
    pub mounts: u64,
    /// Retained views unmounted.
    pub unmounts: u64,
    /// Memo recomputations.
    pub memo_recomputes: u64,
    /// Transactions committed (outermost batch exits).
    pub commits: u64,
    /// Focus moves visible to the user.
    pub focus_moves: u64,
    /// Caret placements.
    pub caret_ops: u64,
    /// Owned title reads at mount (`SPEC-RT-045`).
    pub io_reads: u64,
    /// Store mutations. Unit definition: **one store mutation method call that
    /// changed the model is exactly one write**, and it also advances the store
    /// epoch once. `clear_completed` removing N rows is one write, not N;
    /// `reorder` with the order already in place is zero.
    pub io_writes: u64,
    /// `String` clones performed by the runtime (not by the fixture's `format!`).
    pub string_clones: u64,
}

impl Counters {
    /// Per-field difference, used to report what one scenario step caused.
    pub fn delta(self, earlier: Counters) -> Counters {
        Counters {
            query_evals: self.query_evals.saturating_sub(earlier.query_evals),
            id_materializations: self
                .id_materializations
                .saturating_sub(earlier.id_materializations),
            cache_builds: self.cache_builds.saturating_sub(earlier.cache_builds),
            row_visits: self.row_visits.saturating_sub(earlier.row_visits),
            mounts: self.mounts.saturating_sub(earlier.mounts),
            unmounts: self.unmounts.saturating_sub(earlier.unmounts),
            memo_recomputes: self.memo_recomputes.saturating_sub(earlier.memo_recomputes),
            commits: self.commits.saturating_sub(earlier.commits),
            focus_moves: self.focus_moves.saturating_sub(earlier.focus_moves),
            caret_ops: self.caret_ops.saturating_sub(earlier.caret_ops),
            io_reads: self.io_reads.saturating_sub(earlier.io_reads),
            io_writes: self.io_writes.saturating_sub(earlier.io_writes),
            string_clones: self.string_clones.saturating_sub(earlier.string_clones),
        }
    }

    /// Single number for trace summaries.
    pub fn total(self) -> u64 {
        self.query_evals
            + self.id_materializations
            + self.cache_builds
            + self.row_visits
            + self.mounts
            + self.unmounts
            + self.memo_recomputes
            + self.commits
            + self.focus_moves
            + self.caret_ops
            + self.io_reads
            + self.io_writes
            + self.string_clones
    }
}

impl AddAssign for Counters {
    fn add_assign(&mut self, other: Counters) {
        self.query_evals += other.query_evals;
        self.id_materializations += other.id_materializations;
        self.cache_builds += other.cache_builds;
        self.row_visits += other.row_visits;
        self.mounts += other.mounts;
        self.unmounts += other.unmounts;
        self.memo_recomputes += other.memo_recomputes;
        self.commits += other.commits;
        self.focus_moves += other.focus_moves;
        self.caret_ops += other.caret_ops;
        self.io_reads += other.io_reads;
        self.io_writes += other.io_writes;
        self.string_clones += other.string_clones;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_reports_only_the_step() {
        let earlier = Counters {
            mounts: 3,
            io_reads: 3,
            ..Counters::default()
        };
        let now = Counters {
            mounts: 5,
            io_reads: 3,
            ..Counters::default()
        };
        let delta = now.delta(earlier);
        assert_eq!(delta.mounts, 2);
        assert_eq!(delta.io_reads, 0);
        assert_eq!(delta.total(), 2);
    }

    #[test]
    fn add_assign_accumulates_every_field() {
        let one = Counters {
            query_evals: 1,
            id_materializations: 2,
            cache_builds: 12,
            row_visits: 13,
            mounts: 3,
            unmounts: 4,
            memo_recomputes: 5,
            commits: 6,
            focus_moves: 7,
            caret_ops: 8,
            io_reads: 9,
            io_writes: 10,
            string_clones: 11,
        };
        let two = Counters {
            query_evals: 10,
            id_materializations: 20,
            cache_builds: 120,
            row_visits: 130,
            mounts: 30,
            unmounts: 40,
            memo_recomputes: 50,
            commits: 60,
            focus_moves: 70,
            caret_ops: 80,
            io_reads: 90,
            io_writes: 100,
            string_clones: 110,
        };

        let mut total = Counters::default();
        total += one;
        total += two;

        assert_eq!(total.query_evals, 11);
        assert_eq!(total.id_materializations, 22);
        assert_eq!(total.mounts, 33);
        assert_eq!(total.unmounts, 44);
        assert_eq!(total.memo_recomputes, 55);
        assert_eq!(total.commits, 66);
        assert_eq!(total.focus_moves, 77);
        assert_eq!(total.caret_ops, 88);
        assert_eq!(total.io_reads, 99);
        assert_eq!(total.io_writes, 110);
        assert_eq!(total.string_clones, 121);
    }
}
