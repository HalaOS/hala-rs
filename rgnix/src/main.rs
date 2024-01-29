use hala_rs::future::executor::block_on;

mod rproxy;
use rproxy::*;

fn main() {
    pretty_env_logger::init_timed();

    if let Err(err) = block_on(rproxy_main()) {
        log::error!("rproxy stopped with error: {}", err);
    } else {
        log::error!("rproxy stopped.");
    }
}
