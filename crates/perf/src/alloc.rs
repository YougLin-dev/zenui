//! 计数分配器：门禁按 turn 读取分配次数。
//!
//! 只在前台性能工具里启用；它给每次分配加一次 relaxed 原子加，时间基线
//! 与基线文件来自同一个二进制，因此趋势仍然可比。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

struct CountingAllocator;

// SAFETY: 每个钩子都原样转发给 `System`，布局与指针约束由调用方按
// `GlobalAlloc` 契约提供；这里只增加计数，不改变分配行为。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: 由 `GlobalAlloc::alloc` 的调用方保证 `layout` 合法。
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: 由 `GlobalAlloc::alloc_zeroed` 的调用方保证 `layout` 合法。
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` 由同一分配器按 `layout` 分配，调用方保证其仍然有效。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `ptr` 由同一分配器按 `layout` 分配，`new_size` 是新布局大小；
        // 调用方保证重分配契约成立。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

/// 进程启动以来的分配调用次数（分配与重分配各算一次）。
pub fn allocations() -> u64 {
    ALLOCATIONS.load(Ordering::Relaxed)
}
