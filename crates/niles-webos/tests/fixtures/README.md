# Loopback TLS fixture

`server.der` and `server-key.der` are a self-signed certificate and test-only private key used by the local webOS transport tests. They are not credentials for a TV or a deployed service. Never reuse this key outside tests.

The tests run local WebSocket/TLS servers and do not contact household devices. Legacy mode is verified using plain loopback WebSockets; the TLS path retains the production signature verifier.
