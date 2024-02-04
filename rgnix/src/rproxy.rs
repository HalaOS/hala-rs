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

use std::{
    fs::{self, create_dir_all},
    io, process,
};

use clap::Parser;
use hala_rs::{
    io::sleep,
    pprof::{
        alloc::{create_heap_profiling, get_heap_profiling, HeapProfilingAlloc},
        pprof::HeapProfilingPerfToolsBuilder,
    },
    rproxy::{profile::get_profile_config, GatewayFactoryManager},
};

#[global_allocator]
static ALLOC: HeapProfilingAlloc = HeapProfilingAlloc;

pub async fn rproxy_main() -> io::Result<()> {
    let rproxy_config = ReverseProxy::parse();

    if rproxy_config.pprof {
        create_heap_profiling(None);
    }

    get_profile_config().on(true);

    let profile_interval = rproxy_config.profile_interval.clone();

    let tunnel_factory_manager = create_tunnel_factory_manager(&rproxy_config);

    let gateway_factory_manager = GatewayFactoryManager::new(tunnel_factory_manager.clone());

    let gateway_factory_id = create_gateway_factory(&gateway_factory_manager, &rproxy_config);

    let gateway_id = gateway_factory_manager
        .start(
            &gateway_factory_id,
            create_protocol_config(rproxy_config.clone())?,
        )
        .await?;

    log::info!("Gateway {} created", gateway_id);

    let heap_profiling = get_heap_profiling();

    let mut current_memeory_allocated = if rproxy_config.pprof {
        if !rproxy_config.pprof_dir.exists() {
            create_dir_all(rproxy_config.pprof_dir.clone())?;
        }
        heap_profiling.record(true);
        Some(heap_profiling.allocated())
    } else {
        None
    };

    let mut gateway_profile: ReverseProxyProfile = ReverseProxyProfile::default();

    let mut tunnel_profile = ReverseProxyProfile::default();

    let mut i = 0;
    let process_id = process::id();

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

        if let Some(current) = current_memeory_allocated {
            log::info!("heap profiling:");

            let allocated = get_heap_profiling().allocated();
            let blocks = get_heap_profiling().allocated_blocks();

            if allocated > current && allocated - current > rproxy_config.pprof_memory_increases {
                use hala_rs::pprof::protobuf::Message;

                current_memeory_allocated = Some(allocated);

                let mut report = heap_profiling
                    .report(|| HeapProfilingPerfToolsBuilder::new())
                    .unwrap();

                let profile = report.build();

                let profile_file_path = rproxy_config
                    .pprof_dir
                    .join(format!("heap-{}-{}.pb", process_id, i));

                fs::write(profile_file_path, profile.write_to_bytes().unwrap()).unwrap();

                i += 1;
            }

            log::info!(
                "heap profiling: before={}, current={}, blocks={}",
                current,
                allocated,
                blocks
            );
        }
    }
}
