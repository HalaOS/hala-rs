export RUST_LOG=info

nohup cargo run --manifest-path ../apps/Cargo.toml --release --bin qtun-server -- --laddrs 0.0.0.0:2000-3000 --raddrs 127.0.0.1:12948 --ca-file ./cert/hala_ca.pem --cert-chain-file ./cert/server.crt --key-file ./cert/server.key > server.log &