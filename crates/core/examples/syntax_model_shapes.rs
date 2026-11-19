#![forbid(unsafe_code)]

// These signatures demonstrate identity/capture shape; no resolver or
// scheduler stands behind the example's slot marker.
use std::marker::PhantomData;

struct Signal<T>(
    u32,
    // The markers decouple copying a handle from the data it identifies.
    PhantomData<fn() -> T>,
    PhantomData<*const ()>,
);

impl<T> Copy for Signal<T> {}
impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Signal<T> {
    fn identity(self) -> u32 {
        self.0
    }
}

fn signal<T>(_value: T) -> Signal<T> {
    Signal(0, PhantomData, PhantomData)
}

#[derive(Clone, Copy)]
struct AppState {
    draft: Signal<String>,
    filter: Signal<u32>,
}

#[derive(Debug)]
struct MountPlan;
#[derive(Debug)]
struct MountError;
#[derive(Debug)]
struct InvalidHandle;

type MountResult<T> = Result<T, MountError>;

impl From<InvalidHandle> for MountError {
    fn from(_: InvalidHandle) -> Self {
        Self
    }
}

fn read_once() -> Result<String, InvalidHandle> {
    Ok(String::from("task"))
}

fn todo_item() -> MountResult<MountPlan> {
    let title = read_once()?;
    if title.is_empty() {
        return Err(MountError);
    }
    Ok(MountPlan)
}

#[allow(dead_code)]
fn todo_item_with_local_wrapper() -> MountResult<MountPlan> {
    (|| -> MountResult<MountPlan> {
        let title = read_once()?;
        if title.is_empty() {
            return Err(MountError);
        }
        Ok(MountPlan)
    })()
}

#[derive(Clone, Copy)]
struct CanvasContent(u32);

fn main() {
    let state = AppState {
        draft: signal(String::from("draft")),
        filter: signal(0),
    };

    let factory = move || {
        let local = state;
        let read_draft = move || local.draft;
        let read_filter = move || local.filter;
        (read_draft(), read_filter())
    };
    let _ = factory();
    assert_eq!(factory().0.identity(), factory().1.identity());

    let chart = CanvasContent(42);
    let fallback = move || chart.0;
    let streaming = move || chart.0;
    assert_eq!(fallback() + streaming(), 84);
    assert!(todo_item().is_ok());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_copy_values_have_copy_identity_handles() {
        let state = AppState {
            draft: signal(String::from("draft")),
            filter: signal(0),
        };

        let factory = move || {
            let local = state;
            let read_draft = move || local.draft;
            let read_filter = move || local.filter;
            (read_draft(), read_filter())
        };
        assert_eq!(factory().0.identity(), 0);
        assert_eq!(factory().1.identity(), 0);
    }

    #[test]
    fn a_copy_resource_can_feed_two_alternative_move_branches() {
        let chart = CanvasContent(42);
        let fallback = move || chart.0;
        let streaming = move || chart.0;
        assert_eq!(fallback() + streaming(), 84);
    }

    #[test]
    fn component_tail_and_question_mark_keep_mount_result() {
        assert!(todo_item().is_ok());
        assert!(todo_item_with_local_wrapper().is_ok());
    }

    #[test]
    fn explicit_add_result_type_avoids_unconstrained_error_inference() {
        let add = |commit: bool| -> Result<(), String> {
            if !commit {
                return Err(String::from("not committed"));
            }
            Ok(())
        };
        assert!(add(true).is_ok());
    }
}
