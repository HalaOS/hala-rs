use hala_rs::future::executor::block_on;
use rgnix_tunnel::app::*;

fn main() {
    pretty_env_logger::init_timed();

    if let Err(err) = block_on(run_server()) {
        log::error!("{}", err);
    }
}
