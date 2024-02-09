use std::{
    ops::Sub,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};

use hala_sync::{Lockable, SpinMutex};

use super::{frames_to_symbols, get_backtrace, CpuProfilerReport};

struct CpuSample {
    cpu_times: Duration,
    frames: Vec<usize>,
}

struct CpuProfiler {
    samples: SpinMutex<Vec<CpuSample>>,
    flag: AtomicBool,
}

impl CpuProfiler {
    fn new() -> Self {
        Self {
            samples: Default::default(),
            flag: Default::default(),
        }
    }

    fn start(&self) {
        self.flag
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .expect("calling `cpu_profiler_start` again without calling `cpu_profiler_stop`");
    }

    fn report<R: CpuProfilerReport>(&self, report: &mut R) {
        self.flag
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
            .expect("calling `cpu_profiler_stop` again without calling `cpu_profiler_start`");

        let samples = self.samples.lock().drain(..).collect::<Vec<_>>();

        for sample in samples {
            let symbols = frames_to_symbols(&sample.frames);
            report.report_cpu_sample(sample.cpu_times, &symbols);
        }
    }

    fn sample(&self, instant: Instant) {
        if !self.flag.load(Ordering::Relaxed) {
            return;
        }

        let frames = get_backtrace();

        let sample = CpuSample {
            cpu_times: Instant::now().sub(instant),
            frames,
        };

        self.samples.lock().push(sample);
    }
}

static GLOBAL_CPU_PROFILER: OnceLock<CpuProfiler> = OnceLock::new();

fn global_cpu_profiler() -> &'static CpuProfiler {
    GLOBAL_CPU_PROFILER.get_or_init(|| CpuProfiler::new())
}

/// Start the global cpu profiler and record cpu sampling data.
pub fn cpu_profiler_start() {
    global_cpu_profiler().start();
}

/// Stop the global cpu profiler and use provided [`CpuReport`] to generate a cpu profiling report.
pub fn cpu_profiler_stop<R: CpuProfilerReport>(report: &mut R) {
    global_cpu_profiler().report(report)
}

/// Perform a cpu profiler sample.
pub fn cpu_profiler_sample(instant: Instant) {
    global_cpu_profiler().sample(instant)
}
