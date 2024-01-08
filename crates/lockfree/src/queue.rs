use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;

/// A lokfree `FIFO` queue implementation.
pub struct Queue<T> {
    header: AtomicUsize,
    tail: AtomicUsize,
    slots: DashMap<usize, T>,
}

unsafe impl<T: Send> Send for Queue<T> {}
unsafe impl<T: Send> Sync for Queue<T> {}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            header: Default::default(),
            tail: Default::default(),
            slots: DashMap::new(),
        }
    }
}

impl<T> Queue<T> {
    /// Create new instance of type [`Queue`]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one value into queue tail
    pub fn push(&self, value: T) {
        let offset = self.tail.fetch_add(1, Ordering::AcqRel);

        self.slots.insert(offset, value);
    }

    /// Pop one value from the queue's header. returns [`None`] if this queue is empty.
    pub fn pop(&self) -> Option<T> {
        loop {
            let header = self.header.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);

            if header == tail {
                return None;
            }

            if self
                .header
                .compare_exchange(header, header + 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Some(self.spin_pop_by_offset(header));
            }
        }
    }

    #[cold]
    fn spin_pop_by_offset(&self, index: usize) -> T {
        loop {
            if let Some((_, value)) = self.slots.remove(&index) {
                return value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{atomic::AtomicI32, Arc},
    };

    use super::*;

    #[test]
    fn test_mulit_push() {
        let queue = Arc::new(Queue::<i32>::new());
        let counter = Arc::new(AtomicI32::new(0));

        let push_threads = 10;
        let loops = 10000;

        for _ in 0..push_threads {
            let queue = queue.clone();
            let counter = counter.clone();
            std::thread::spawn(move || {
                for _ in 0..loops {
                    queue.push(counter.fetch_add(1, Ordering::AcqRel));
                }
            });
        }

        let mut set = HashSet::new();

        for _ in 0..push_threads * loops {
            'inner: loop {
                if let Some(value) = queue.pop() {
                    set.insert(value);
                    break 'inner;
                }
            }
        }

        assert_eq!(set.len() as i32, push_threads * loops);

        for i in 0..push_threads * loops {
            assert!(set.contains(&i), "Not found {i}");
        }
    }

    #[test]
    fn test_mulit_push_pop() {
        let queue = Arc::new(Queue::<i32>::new());
        let counter = Arc::new(AtomicI32::new(0));
        let pop_counter = Arc::new(AtomicI32::new(0));

        let push_threads = 5;
        let pop_threads = 5;
        let loops = 100000;

        for _ in 0..push_threads {
            let queue = queue.clone();
            let counter = counter.clone();
            std::thread::spawn(move || {
                for _ in 0..loops {
                    queue.push(counter.fetch_add(1, Ordering::AcqRel));
                }
            });
        }

        let mut handles = vec![];

        for _ in 0..pop_threads {
            let queue = queue.clone();
            let pop_counter = pop_counter.clone();
            handles.push(std::thread::spawn(move || {
                while queue.header.load(Ordering::Acquire) < push_threads * loops {
                    if let Some(_) = queue.pop() {
                        pop_counter.fetch_add(1, Ordering::AcqRel);
                    }
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(
            pop_counter.load(Ordering::Relaxed),
            counter.load(Ordering::Relaxed)
        );
    }
}
