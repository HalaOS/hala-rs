use divan::{bench, Bencher};

fn main() {
    divan::main();
}

#[bench(threads)]
fn allo_string(bench: Bencher) {
    bench.bench(|| {
        divan::black_box({
            let _s = "hello world".to_string();
        })
    });
}
