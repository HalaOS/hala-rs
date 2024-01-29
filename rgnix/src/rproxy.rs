mod jemalloc;

mod utils;
pub use utils::*;

mod claps;
pub use claps::*;

mod tcp;
pub use tcp::*;

mod quic;
pub use quic::*;

mod gateway;
pub use gateway::*;

mod tunnel;
pub use tunnel::*;

mod profile;
pub use profile::*;

use std::io;

use clap::Parser;
use hala_rs::{
    io::sleep,
    rproxy::{profile::get_profile_config, GatewayFactoryManager},
};

pub async fn rproxy_main() -> io::Result<()> {
    let rproxy_config = ReverseProxy::parse();

    get_profile_config().on(true);

    let profile_interval = rproxy_config.profile_interval.clone();

    let tunnel_factory_manager = create_tunnel_factory_manager(&rproxy_config);

    let gateway_factory_manager = GatewayFactoryManager::new(tunnel_factory_manager.clone());

    let gateway_factory_id = create_gateway_factory(&gateway_factory_manager, &rproxy_config);

    let gateway_id = gateway_factory_manager
        .start(&gateway_factory_id, create_protocol_config(rproxy_config)?)
        .await?;

    log::info!("Gateway {} created", gateway_id);

    let mut gateway_profile: ReverseProxyProfile = ReverseProxyProfile::default();

    let mut tunnel_profile = ReverseProxyProfile::default();

    loop {
        sleep(profile_interval).await.unwrap();

        let samples = gateway_factory_manager.sample();

        for sample in samples {
            if sample.is_gateway {
                gateway_profile.update(sample);
            } else {
                tunnel_profile.update(sample);
            }
        }

        log::info!("gateway: {:?}", gateway_profile);
        log::info!("tunnel: {:?}", tunnel_profile);
    }
}
