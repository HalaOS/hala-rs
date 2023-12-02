use divan::Bencher;
use hala_io_driver::*;

fn main() {
    // Run registered benchmarks.
    divan::main();
}

#[derive(Clone)]
struct MockDriver {}

struct MockFile {}

impl RawDriver for MockDriver {
    #[inline(never)]
    fn fd_open(
        &self,
        desc: crate::Description,
        _open_flags: crate::OpenFlags,
    ) -> std::io::Result<crate::Handle> {
        Ok(Handle::from((desc, MockFile {})))
    }

    #[inline(never)]
    fn fd_cntl(&self, _handle: crate::Handle, _cmd: crate::Cmd) -> std::io::Result<crate::CmdResp> {
        Ok(CmdResp::None)
    }

    #[inline(never)]
    fn fd_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        handle.drop_as::<MockFile>();

        Ok(())
    }
}

#[test]
fn test_driver_vtable() {
    let driver = Driver::new(MockDriver {});

    let driver = driver.clone();

    let handle = driver.fd_open(Description::File, OpenFlags::None).unwrap();

    driver.fd_cntl(handle, Cmd::PollOnce(None)).unwrap();

    driver.fd_close(handle).unwrap();
}

#[divan::bench(sample_size = 10000)]
fn vtable_call(bench: Bencher) {
    let driver = Driver::new(MockDriver {});

    bench.bench_local(|| {
        let handle = driver.fd_open(Description::File, OpenFlags::None).unwrap();

        driver.fd_cntl(handle, Cmd::PollOnce(None)).unwrap();

        driver.fd_close(handle).unwrap();
    });
}

#[divan::bench(sample_size = 10000)]
fn direct_call(bench: Bencher) {
    let driver = MockDriver {};

    bench.bench_local(|| {
        let handle = driver.fd_open(Description::File, OpenFlags::None).unwrap();

        driver.fd_cntl(handle, Cmd::PollOnce(None)).unwrap();

        driver.fd_close(handle).unwrap();
    });
}
