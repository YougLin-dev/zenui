//! renderer 的 per-tree 呈现视图：持有最新快照根并跟踪已应用的 revision。
//!
//! `applied_revision` 只存在于消费方，不在 runtime。delta 只在
//! `from_revision == applied_revision` 且与携带它的快照同代时被接受；断档时
//! 丢弃 delta，按最新根全量重建——这是合法回退，不是错误。UIA 不消费
//! delta，也不维护 `applied_revision`。

use zenui::{PresentationSnapshot, PresentationUpdate};

/// 一次 [`PresentationView::apply`] 的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationApply {
    /// delta 与已应用 revision 连续；变化键可用于失效增量缓存。
    Applied,
    /// 完整根或 delta 断档：丢弃 delta，按最新根全量重建。
    FullRebuild,
}

/// renderer 的 per-tree 呈现视图；Clone 共享快照根。
#[derive(Clone, Debug)]
pub struct PresentationView {
    snapshot: PresentationSnapshot,
    applied_revision: u64,
}

impl PresentationView {
    /// 从完整根开始；首次发布或强制全量重建使用。
    pub fn new(snapshot: PresentationSnapshot) -> Self {
        let applied_revision = snapshot.revision();
        Self {
            snapshot,
            applied_revision,
        }
    }

    /// 当前已应用的快照根；场景构建只读取它。
    pub fn snapshot(&self) -> &PresentationSnapshot {
        &self.snapshot
    }

    /// renderer 已应用的 revision；宿主跳过 `from == applied` 不成立的 delta。
    pub const fn applied_revision(&self) -> u64 {
        self.applied_revision
    }

    /// 应用一次发布。
    ///
    /// 连续 delta 返回 [`PresentationApply::Applied`]；`delta: None`、断档或
    /// delta 与快照不同代时返回 [`PresentationApply::FullRebuild`]。两种情况
    /// 都采用发布根作为新视图，快照根始终是权威数据。
    pub fn apply(&mut self, update: PresentationUpdate) -> PresentationApply {
        if update.snapshot().revision() == self.applied_revision {
            // 同一发布根：没有新 revision 可应用，也不需要重建。
            self.snapshot = update.into_snapshot();
            return PresentationApply::Applied;
        }
        let contiguous = update.delta().is_some_and(|delta| {
            delta.from_revision() == self.applied_revision
                && delta.to_revision() == update.snapshot().revision()
        });
        self.applied_revision = update.snapshot().revision();
        self.snapshot = update.into_snapshot();
        if contiguous {
            PresentationApply::Applied
        } else {
            PresentationApply::FullRebuild
        }
    }
}
