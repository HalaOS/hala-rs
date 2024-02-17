
openssl req -new -x509 -batch -nodes -days 10000 -keyout hala_ca.key -out hala_ca.pem

openssl req -new -batch -nodes -sha256 -keyout server.key -out server.csr -subj '/C=GB/CN=hala.quic'
openssl x509 -req -days 10000 -in server.csr -CA hala_ca.pem -CAkey hala_ca.key -CAcreateserial -out server.crt
openssl verify -CAfile hala_ca.pem server.crt

openssl req -new -batch -nodes -sha256 -keyout client.key -out client.csr -subj '/C=GB/CN=hala.quic'
openssl x509 -req -days 10000 -in client.csr -CA hala_ca.pem -CAkey hala_ca.key -CAcreateserial -out client.crt
openssl verify -CAfile hala_ca.pem client.crt



rm client.csr
rm server.csr
rm hala_ca.key
rm hala_ca.srl

