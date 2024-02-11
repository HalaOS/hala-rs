RUST_LOG=info

nohup cargo run --bin rproxy -- --laddrs 0.0.0.0:1812 -g tcp -t quic --raddrs 127.0.0.1:2000-2010 --tunnel-ca-file ./cert/hala_ca.pem --verify-server --tunnel-cert-chain-file ./cert/client.crt --tunnel-key-file ./cert/client.key > client.log &
