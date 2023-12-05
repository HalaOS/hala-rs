#![no_std]
extern crate alloc;

use core::task::Poll;

use alloc::collections::{BTreeMap, VecDeque};

struct Slot<T> {
    round: u64,
    t: T,
}

pub struct TimeWheel<T> {
    hashed: BTreeMap<u64, VecDeque<Slot<T>>>,
    steps: u64,
    tick: u64,
}

impl<T> TimeWheel<T> {
    // create new hashed time wheel instance
    pub fn new(steps: u64) -> Self {
        TimeWheel {
            steps: steps,
            hashed: BTreeMap::new(),
            tick: 0,
        }
    }

    pub fn add(&mut self, timeout: u64, value: T) {
        let slot = (timeout + self.tick) % self.steps;

        let slots = self.hashed.entry(slot).or_insert(Default::default());

        slots.push_back(Slot {
            t: value,
            round: (timeout + self.tick) / self.steps,
        });
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
