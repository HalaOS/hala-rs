export RUST_LOG=info

export MALLOC_CONF="prof:true,log_prof_interval:20,prof_prefix:jeprof.out"

nohup cargo run --example rproxy -- --laddrs 127.0.0.1:1812 -g tcp -t quic --peer-domain 127.0.0.1 --peer-port-range 1813 --tunnel-cert-chain-file ./cert/client.crt --tunnel-key-file ./cert/client.key > client.log &

nohup cargo run --example rproxy -- --laddrs 127.0.0.1:1813 -g quic -t tcp --peer-domain 127.0.0.1 --peer-port-range 12948 --gateway-cert-chain-file ./cert/server.crt --gateway-key-file ./cert/server.key > server.log &