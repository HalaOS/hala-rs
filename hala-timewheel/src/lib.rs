#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

use core::task::Poll;

#[cfg(not(feature = "std"))]
use alloc::collections::{BTreeMap, VecDeque};

#[cfg(feature = "std")]
use std::collections::{HashMap, VecDeque};

struct Slot<T> {
    round: u128,
    t: T,
}

pub struct TimeWheel<T> {
    #[cfg(not(feature = "std"))]
    hashed: BTreeMap<u128, VecDeque<Slot<T>>>,
    #[cfg(feature = "std")]
    hashed: HashMap<u128, VecDeque<Slot<T>>>,
    steps: u128,
    tick: u128,
}

impl<T> TimeWheel<T> {
    // create new hashed time wheel instance
    pub fn new(steps: u128) -> Self {
        TimeWheel {
            steps: steps,
            hashed: Default::default(),
            tick: 0,
        }
    }

    pub fn add(&mut self, timeout: u128, value: T) -> u128 {
        let slot = (timeout + self.tick) % self.steps;

        let slots = self.hashed.entry(slot).or_insert(Default::default());

        slots.push_back(Slot {
            t: value,
            round: (timeout + self.tick) / self.steps,
        });

        slot
    }

    pub fn remove(&mut self, slot: u128, value: T) -> bool
    where
        T: PartialEq,
    {
        if let Some(slots) = self.hashed.get_mut(&slot) {
            let index =
                slots
                    .iter()
                    .enumerate()
                    .find_map(|(index, v)| if v.t == value { Some(index) } else { None });

            if let Some(index) = index {
                slots.remove(index);
                return true;
            } else {
                return false;
            }
        }

        false
    }

    pub fn tick(&mut self) -> Poll<VecDeque<T>> {
        let step = self.tick % self.steps;

        self.tick += 1;

        if let Some(slots) = self.hashed.remove(&step) {
            let mut current: VecDeque<T> = Default::default();
            let mut reserved: VecDeque<Slot<T>> = Default::default();

            for slot in slots {
                if slot.round == 0 {
                    current.push_back(slot.t);
                } else {
                    reserved.push_back(Slot::<T> {
                        t: slot.t,
                        round: slot.round - 1,
                    });
                }
            }

            self.hashed.insert(step, reserved);

            return Poll::Ready(current);
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use crate::TimeWheel;

    #[test]
    fn test_remove() {
        let mut time_wheel = TimeWheel::new(1024);

        let slot = time_wheel.add(10, 1);

        assert!(time_wheel.remove(slot, 1));

        assert!(!time_wheel.remove(slot, 10));
    }
}
