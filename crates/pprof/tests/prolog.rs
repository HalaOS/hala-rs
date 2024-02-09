use std::panic::catch_unwind;

use hala_pprof::{set_prolog, trace, ProfilingLog};
use hala_pprof_derive::def_target;

def_target!(MOCK_TARGET, "");

struct MockProLog;

impl ProfilingLog for MockProLog {
    fn filter(&self, target: &'static str) -> bool {
        target == MOCK_TARGET
    }

    fn profiling(&self, _target: &'static str, args: &[&dyn std::any::Any]) {
        assert_eq!(*args[0].downcast_ref::<i32>().unwrap(), 1);
    }
}

#[test]
fn prolog_test() {
    set_prolog(MockProLog);
    trace!(target: MOCK_TARGET,"hello {}", 1);

    let f = || {
        trace!(target: MOCK_TARGET,"hello {}", 2);
    };

    catch_unwind(f).expect_err("Expect panic");
}
