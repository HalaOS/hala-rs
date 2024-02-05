use hala_tracing_derive::*;

target!(HELLO_WORLD);

#[test]
fn test_event_def() {
    println!("{}", HELLO_WORLD);
}

#[cpu_profiling]
fn test_a() {
    println!("hello world");
}

struct Mock {}

impl Mock {
    #[cpu_profiling]
    fn test(&self) {}
}

impl MockTrait for Mock {
    #[cpu_profiling]
    fn mock_fn_1(&self) -> String {
        "Hello".to_string()
    }
}

trait MockTrait {
    #[cpu_profiling]
    fn mock_fn(&self) -> i32 {
        1
    }

    fn mock_fn_1(&self) -> String;
}
