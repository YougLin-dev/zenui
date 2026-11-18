// crates/core/src/arena.rs
//! Generational slot storage backing the store's handles.
//!
//! Recycling a slot bumps its generation, so a handle minted before the recycle
//! stops resolving (`SPEC-RT-005` failure mode: "deleted row still returns the
//! previous value"). Slot reuse itself is the `SPEC-GATE-A0-03` subject.

use crate::handle::{Generation, HandleError, SlotIndex};

#[derive(Debug)]
pub(crate) struct Slots<T> {
    values: Vec<Option<T>>,
    generations: Vec<Generation>,
    free: Vec<u32>,
}

impl<T> Slots<T> {
    pub(crate) const fn new() -> Self {
        Self {
            values: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
        }
    }

    pub(crate) fn get(&self, slot: SlotIndex, generation: Generation) -> Result<&T, HandleError> {
        let index = slot.index() as usize;
        let Some(entry) = self.values.get(index) else {
            return Err(HandleError::Vacant { slot });
        };
        let Some(value) = entry.as_ref() else {
            return Err(HandleError::Vacant { slot });
        };
        if self.generations[index] != generation {
            return Err(HandleError::StaleGeneration {
                slot,
                current: self.generations[index],
                handle: generation,
            });
        }
        Ok(value)
    }

    pub(crate) fn get_mut(
        &mut self,
        slot: SlotIndex,
        generation: Generation,
    ) -> Result<&mut T, HandleError> {
        let index = slot.index() as usize;
        let Some(entry) = self.values.get_mut(index) else {
            return Err(HandleError::Vacant { slot });
        };
        let Some(value) = entry.as_mut() else {
            return Err(HandleError::Vacant { slot });
        };
        if self.generations[index] != generation {
            return Err(HandleError::StaleGeneration {
                slot,
                current: self.generations[index],
                handle: generation,
            });
        }
        Ok(value)
    }

    /// Reserve a slot for a transaction without a value yet (`SPEC-RT-051`).
    ///
    /// Reservation is the one eager authority change a transaction makes: the
    /// handle it mints must stay valid after commit, and must never alias a
    /// later row after rollback. `release` burns the generation for that.
    pub(crate) fn reserve(&mut self) -> (SlotIndex, Generation) {
        match self.free.pop() {
            Some(raw) => {
                let index = raw as usize;
                debug_assert!(self.values[index].is_none(), "free slot was occupied");
                (SlotIndex::from_index(raw), self.generations[index])
            }
            None => {
                let raw = u32::try_from(self.values.len()).expect("slot table exhausted");
                self.values.push(None);
                self.generations.push(Generation::FIRST);
                (SlotIndex::from_index(raw), Generation::FIRST)
            }
        }
    }

    /// Fill a reserved slot. Returns `false` if the slot was not reserved.
    pub(crate) fn fill(&mut self, slot: SlotIndex, value: T) -> bool {
        match self.values.get_mut(slot.index() as usize) {
            Some(entry @ None) => {
                *entry = Some(value);
                true
            }
            _ => false,
        }
    }

    /// Drop a reservation and advance the generation, burning its handle.
    pub(crate) fn release(&mut self, slot: SlotIndex) {
        let index = slot.index() as usize;
        if index >= self.values.len() {
            return;
        }
        self.values[index] = None;
        self.generations[index] = self.generations[index].next();
        self.free.push(slot.index());
    }

    /// Free a slot and advance its generation so old handles fail.
    pub(crate) fn free(&mut self, slot: SlotIndex) -> Option<T> {
        let index = slot.index() as usize;
        let value = self.values.get_mut(index)?.take()?;
        self.generations[index] = self.generations[index].next();
        self.free.push(slot.index());
        Some(value)
    }

    /// Find a live slot by value. Used during commit, when the row exists but
    /// has not been placed in the order vector yet.
    pub(crate) fn find<F>(&self, mut predicate: F) -> Option<SlotIndex>
    where
        F: FnMut(&T) -> bool,
    {
        self.values.iter().enumerate().find_map(|(index, entry)| {
            let value = entry.as_ref()?;
            predicate(value).then(|| SlotIndex::from_index(index as u32))
        })
    }

    pub(crate) fn generation(&self, slot: SlotIndex) -> Option<Generation> {
        self.generations.get(slot.index() as usize).copied()
    }

    pub(crate) fn get_ref(&self, slot: SlotIndex) -> Option<&T> {
        self.values.get(slot.index() as usize)?.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recycled_slot_rejects_the_previous_generation() {
        let mut slots = Slots::new();
        let (first_slot, first_generation) = slots.reserve();
        assert!(slots.fill(first_slot, "first"));
        assert!(slots.get(first_slot, first_generation).is_ok());

        slots.free(first_slot);
        let (second_slot, second_generation) = slots.reserve();
        assert!(slots.fill(second_slot, "second"));

        assert_eq!(first_slot, second_slot, "slot is recycled");
        assert_ne!(first_generation, second_generation);
        assert!(slots.get(first_slot, first_generation).is_err());
        assert_eq!(
            slots
                .get(second_slot, second_generation)
                .expect("live slot"),
            &"second"
        );
    }

    #[test]
    fn freed_slot_without_reuse_is_vacant() {
        let mut slots = Slots::new();
        let (slot, generation) = slots.reserve();
        assert!(slots.fill(slot, 7_u32));
        slots.free(slot);
        assert!(matches!(
            slots.get(slot, generation),
            Err(HandleError::Vacant { .. })
        ));
    }
}
