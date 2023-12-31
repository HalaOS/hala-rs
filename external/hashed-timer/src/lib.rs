use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;

/// Lockfree hashed time wheel implementation
#[derive(Debug, Clone)]
pub struct HashedTimeWheel<T> {
    /// The timer map from ticks to timers queue.
    timers: Arc<DashMap<u64, boxcar::Vec<T>>>,
    /// Clock ticks that have elapsed since [``HashedTimeWheel``] was created.
    ticks: Arc<AtomicU64>,
    /// The timestamp of this timewheel instance was created
    start_instant: Instant,
    /// The duration of one tick
    tick_duration: u128,
    /// Aliving timers's count.
    timer_count: Arc<AtomicU64>,
}

impl<T> HashedTimeWheel<T> {
    /// Create new default [`HashedTimeWheel`] instance.
    pub fn new(tick_duration: Duration) -> Self {
        Self {
            timers: Arc::new(DashMap::default()),
            ticks: Default::default(),
            start_instant: Instant::now(),
            tick_duration: tick_duration.as_micros(),
            timer_count: Default::default(),
        }
    }

    /// Returns aliving timers's count
    pub fn timers(&self) -> u64 {
        return self.timer_count.load(Ordering::Relaxed);
    }

    /// Creates a new timer and returns the timer expiration ticks.
    pub fn new_timer(&self, timer: T, duration: Duration) -> Option<u64> {
        let instant_duration = Instant::now() - self.start_instant;

        let ticks = (instant_duration + duration).as_micros() / self.tick_duration;

        let ticks = ticks as u64;

        if self
            .ticks
            .fetch_update(Ordering::Release, Ordering::Acquire, |current| {
                if current > ticks {
                    None
                } else {
                    Some(current)
                }
            })
            .is_err()
        {
            return None;
        }

        self.timers.entry(ticks).or_default().push(timer);

        if self.ticks.load(Ordering::SeqCst) > ticks {
            if self.timers.remove(&ticks).is_some() {
                return None;
            }
        }

        if self
            .ticks
            .fetch_update(Ordering::Release, Ordering::Acquire, |current| {
                if current > ticks {
                    if self.timers.remove(&ticks).is_some() {
                        return None;
                    } else {
                        return Some(current);
                    }
                } else {
                    return Some(current);
                }
            })
            .is_err()
        {
            return None;
        }

        self.timer_count.fetch_add(1, Ordering::SeqCst);

        Some(ticks)
    }

    /// Forward to next tick, and returns timeout timers.
    pub fn next_tick(&self) -> Option<Vec<T>> {
        loop {
            let current = self.ticks.load(Ordering::Acquire);

            let instant_duration = Instant::now() - self.start_instant;

            let ticks = (instant_duration.as_micros() / self.tick_duration) as u64;

            assert!(current <= ticks);

            if current == ticks {
                return None;
            }

            if self
                .ticks
                .compare_exchange(current, ticks, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                let mut timeout_timers = vec![];

                for i in current..ticks {
                    if let Some((_, queue)) = self.timers.remove(&i) {
                        for t in queue.into_iter() {
                            timeout_timers.push(t);
                        }
                    }
                }

                return Some(timeout_timers);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc, Barrier,
        },
        thread::sleep,
        time::Duration,
    };

    use crate::HashedTimeWheel;

    #[test]
    fn test_add_timers() {
        let threads = 10;
        let loops = 10usize;

        let time_wheel = HashedTimeWheel::<i32>::new(Duration::from_millis(100));

        let barrier = Arc::new(Barrier::new(threads));

        let mut handles = vec![];

        for _ in 0..threads {
            let barrier = barrier.clone();

            let time_wheel = time_wheel.clone();

            handles.push(std::thread::spawn(move || {
                barrier.wait();

                for i in 0..loops {
                    time_wheel
                        .new_timer(i as i32, Duration::from_secs((i + 1) as u64))
                        .unwrap();
                }
            }))
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(time_wheel.timers() as usize, threads * loops);

        let mut handles = vec![];

        let counter = Arc::new(AtomicU64::new(0));

        for _ in 0..threads {
            let time_wheel = time_wheel.clone();

            let counter = counter.clone();

            handles.push(std::thread::spawn(move || loop {
                if let Some(timers) = time_wheel.next_tick() {
                    counter.fetch_add(timers.len() as u64, Ordering::SeqCst);
                }

                if counter.load(Ordering::SeqCst) == (threads * loops) as u64 {
                    break;
                }
            }))
        }

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_next_tick() {
        let time_wheel = HashedTimeWheel::<i32>::new(Duration::from_millis(100));
        assert_eq!(time_wheel.new_timer(1, Duration::from_millis(100)), Some(1));

        sleep(Duration::from_millis(200));

        assert_eq!(time_wheel.next_tick(), Some(vec![1]));
    }
}
