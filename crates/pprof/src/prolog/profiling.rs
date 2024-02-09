use std::{any::Any, sync::OnceLock};

pub trait ProfilingLog {
    fn filter(&self, target: &'static str) -> bool;
    fn profiling(&self, target: &'static str, args: &[&dyn Any]);
}

static PROFILING_LOG: OnceLock<Box<dyn ProfilingLog + Sync + Send>> = OnceLock::new();

pub fn set_prolog<P: ProfilingLog + Sync + Send + 'static>(log: P) {
    if PROFILING_LOG.set(Box::new(log)).is_err() {
        panic!("Call init_profiling_log more than once")
    }
}

/// filter profiling target.
#[inline(always)]
pub fn prolog_filter(target: &'static str) -> bool {
    if let Some(log) = PROFILING_LOG.get() {
        log.filter(target)
    } else {
        false
    }
}

#[inline(always)]
pub fn prolog_write(target: &'static str, args: &[&dyn Any]) {
    if let Some(log) = PROFILING_LOG.get() {
        log.profiling(target, args)
    }
}
