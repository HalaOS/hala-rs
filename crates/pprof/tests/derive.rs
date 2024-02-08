use hala_pprof::*;

#[test]
fn test_macros() {
    trace!(target: "my_target", "hello");
    error!(target: "my_target", "hello {}",1);
    debug!(target: "my_target", "hello {}",1);
}

#[cpu_profiling]
fn test_cpu_profiling() {}
