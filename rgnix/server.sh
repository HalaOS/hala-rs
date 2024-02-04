export RUST_LOG=info

nohup cargo run --bin rgnix -- --laddrs 0.0.0.0:2000-2010 -g quic -t tcp --raddrs 127.0.0.1:12948 --gateway-ca-file ./cert/hala_ca.pem --verify-client --gateway-cert-chain-file ./cert/server.crt --gateway-key-file ./cert/server.key > server.log &