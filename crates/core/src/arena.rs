use crate::handle::{Generation, HandleError, SlotIndex};

#[derive(Debug)]
pub(crate) struct Slots<T> {
    values: Vec<Option<T>>,
    generations: Vec<Generation>,
    free: Vec<SlotIndex>,
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

    pub(crate) fn reserve(&mut self) -> (SlotIndex, Generation) {
        match self.free.pop() {
            Some(slot) => {
                debug_assert!(
                    self.values[slot.index() as usize].is_none(),
                    "free slot was occupied"
                );
                (slot, self.generations[slot.index() as usize])
            }
            None => {
                let raw = u32::try_from(self.values.len()).expect("slot table exhausted");
                self.values.push(None);
                self.generations.push(Generation::FIRST);
                (SlotIndex::from_index(raw), Generation::FIRST)
            }
        }
    }

    pub(crate) fn fill(&mut self, slot: SlotIndex, value: T) -> bool {
        match self.values.get_mut(slot.index() as usize) {
            Some(entry @ None) => {
                *entry = Some(value);
                true
            }
            _ => false,
        }
    }

    pub(crate) fn release(&mut self, slot: SlotIndex) {
        self.retire(slot);
    }

    pub(crate) fn free(&mut self, slot: SlotIndex) -> Option<T> {
        let value = self.values.get_mut(slot.index() as usize)?.take()?;
        self.retire(slot);
        Some(value)
    }

    fn retire(&mut self, slot: SlotIndex) {
        let index = slot.index() as usize;
        if index >= self.values.len() || self.generations[index] == Generation::retiring() {
            return;
        }
        self.values[index] = None;
        let released_generation = self.generations[index];
        let retired = released_generation.raw() == Generation::LAST_ALLOCATABLE_RAW;
        self.generations[index] = released_generation.next();
        if !retired {
            self.free.push(slot);
        }
    }

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
    fn an_exhausted_slot_is_retired_instead_of_reused() {
        let mut slots = Slots::<&str> {
            values: vec![None, None],
            generations: vec![
                Generation::from_raw(Generation::LAST_ALLOCATABLE_RAW),
                Generation::FIRST,
            ],
            free: vec![SlotIndex::from_index(0)],
        };

        let (last_slot, last_generation) = slots.reserve();
        assert!(slots.fill(last_slot, "last"));
        assert_eq!(slots.get(last_slot, last_generation), Ok(&"last"));
        slots.free(last_slot);
        assert_eq!(slots.generation(last_slot), Some(Generation::retiring()));

        let (fresh_slot, fresh_generation) = slots.reserve();
        assert!(slots.fill(fresh_slot, "fresh"));
        slots.release(fresh_slot);

        assert_ne!(fresh_slot, last_slot);
        assert_ne!(fresh_generation, last_generation);
        assert_eq!(slots.generation(last_slot), Some(Generation::retiring()));
        assert_eq!(slots.generation(fresh_slot), Some(Generation::FIRST.next()));
        assert!(!slots.free.contains(&last_slot));
        assert!(slots.free.contains(&fresh_slot));
        assert!(matches!(
            slots.get(last_slot, last_generation),
            Err(HandleError::Vacant { .. })
        ));
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
