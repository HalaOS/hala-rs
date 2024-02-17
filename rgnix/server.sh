export RUST_LOG=info

nohup cargo run --release --features="server" --bin qtun-server -- --laddrs 0.0.0.0:2000-2010 --raddrs 127.0.0.1:12948 --ca-file ./cert/hala_ca.pem --cert-chain-file ./cert/server.crt --key-file ./cert/server.key > server.log &