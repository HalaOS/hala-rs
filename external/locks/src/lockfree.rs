use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;

#[derive(Debug)]
pub struct Queue<T> {
    map: DashMap<usize, T>,
    /// The offset of the header of this queue.
    head: AtomicUsize,
    /// The offset of the tail of this queue.
    tail: AtomicUsize,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            map: Default::default(),
            head: Default::default(),
            tail: Default::default(),
        }
    }
}

impl<T> Queue<T> {
    /// Creates a new quene with a specified starting capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            map: DashMap::with_capacity(capacity),
            ..Default::default()
        }
    }

    /// Push new value into the tail of this [`Queue`].
    pub fn push(&self, value: T) {
        let index = self.tail.fetch_add(1, Ordering::AcqRel);

        self.map.insert(index, value);
    }

    /// Pop one value from the header of this [`Queue`].
    pub fn pop(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            let tail = self.tail.load(Ordering::Acquire);

            if head == tail {
                return None;
            }

            if self
                .head
                .compare_exchange(head, head + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }

            return self.map.remove(&head).map(|(_, v)| v);
        }
    }

    pub fn len(&self) -> usize {
        self.tail.load(Ordering::Acquire) - self.head.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::Queue;

    #[test]
    fn test_queue() {
        let threads = 10usize;
        let queue = Arc::new(Queue::<usize>::default());

        let mut join_handles = vec![];

        for _ in 0..threads {
            let queue = queue.clone();

            join_handles.push(std::thread::spawn(move || {
                for i in 0..threads {
                    queue.push(i);
                }
            }));
        }

        for handle in join_handles {
            handle.join().unwrap();
        }

        assert_eq!(queue.len(), threads * threads);

        let mut join_handles = vec![];

        for _ in 0..threads {
            let queue = queue.clone();

            join_handles.push(std::thread::spawn(move || {
                for _ in 0..threads {
                    queue.pop().expect("Pop data");
                }
            }));
        }

        for handle in join_handles {
            handle.join().unwrap();
        }
    }
}
