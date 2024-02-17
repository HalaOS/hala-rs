RUST_LOG=info

nohup cargo run --features="client" --bin qtun-client -- --laddrs 0.0.0.0:1812 --raddrs 127.0.0.1:2000-2010 --ca-file ./cert/hala_ca.pem --cert-chain-file ./cert/client.crt --key-file ./cert/client.key > client.log &
