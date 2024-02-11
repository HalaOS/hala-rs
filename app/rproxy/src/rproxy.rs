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

use std::{
    fs::{self, create_dir_all},
    io, process,
};

use clap::Parser;
use hala_rs::{
    io::sleep,
    pprof::profiler::{
        gperf::GperfHeapProfilerReport, heap_profiler_report, heap_profiler_start,
        heap_profiler_stats, HeapProfilerAlloc,
    },
    rproxy::GatewayFactoryManager,
};

#[global_allocator]
static ALLOC: HeapProfilerAlloc = HeapProfilerAlloc;

pub async fn rproxy_main() -> io::Result<()> {
    let rproxy_config = ReverseProxy::parse();

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

    let mut heap_stats = if rproxy_config.pprof {
        heap_profiler_start();

        if !rproxy_config.pprof_dir.exists() {
            create_dir_all(rproxy_config.pprof_dir.clone())?;
        }

        Some(heap_profiler_stats())
    } else {
        None
    };

    let mut i = 0;
    let process_id = process::id();

    loop {
        sleep(profile_interval).await.unwrap();

        if !rproxy_config.pprof {
            continue;
        }

        if let Some(prev_stats) = heap_stats.clone() {
            log::info!("heap profiling:");

            let stats = heap_profiler_stats();

            if stats.memory_size > prev_stats.memory_size
                && stats.memory_size - prev_stats.memory_size > rproxy_config.pprof_memory_increases
            {
                heap_stats = Some(stats.clone());

                let mut gperf_report = GperfHeapProfilerReport::new();

                heap_profiler_report(&mut gperf_report);

                let profile = gperf_report.build();

                let profile_file_path = rproxy_config
                    .pprof_dir
                    .join(format!("heap-{}-{}.pb", process_id, i));

                use hala_rs::pprof::protobuf::Message;

                fs::write(profile_file_path, profile.write_to_bytes().unwrap()).unwrap();

                i += 1;
            }

            log::info!("heap profiling: prev={:?}, current={:?}", prev_stats, stats);
        }
    }
}
