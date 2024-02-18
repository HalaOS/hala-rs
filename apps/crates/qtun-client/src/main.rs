use hala_qtun::app::*;
use hala_rs::future::executor::block_on;

fn main() {
    pretty_env_logger::init_timed();

    if let Err(err) = block_on(run_client()) {
        log::error!("{}", err);
    }
}
