use hala_tracing_derive::target;

target!(HELLO_WORLD);

#[test]
fn test_event_def() {
    println!("{}", HELLO_WORLD);
}
