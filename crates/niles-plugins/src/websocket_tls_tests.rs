use super::*;
use rustls::pki_types::PrivatePkcs8KeyDer;

async fn handshake(trust: bool, name: &str) {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    if trust {
        roots.add(cert.der().clone()).unwrap();
    }
    let server_config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.der().clone()],
        PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let success = trust && name == "localhost";
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tls = tokio_rustls::TlsAcceptor::from(Arc::new(server_config))
            .accept(tcp)
            .await;
        if success {
            let mut socket = tokio_tungstenite::accept_async(tls.unwrap()).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            assert_eq!(message, Message::Text("tls echo".into()));
            socket.send(message).await.unwrap();
        } else {
            assert!(tls.is_err());
        }
    });
    let tcp = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!("wss://{name}:{}/events?test=1", address.port());
    let result = tokio_tungstenite::client_async_tls_with_config(
        request,
        tcp,
        None,
        Some(tls_connector(roots)),
    )
    .await;
    if success {
        let (mut socket, _) = result.unwrap();
        socket.send(Message::Text("tls echo".into())).await.unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Text("tls echo".into())
        );
    } else {
        let (kind, _) = classify_handshake_error(result.unwrap_err());
        assert_eq!(kind, FailureKind::Tls);
    }
    server.await.unwrap();
}

#[tokio::test]
async fn real_wss_echo_with_explicit_crypto_provider() {
    tokio::time::timeout(Duration::from_secs(5), handshake(true, "localhost"))
        .await
        .unwrap();
}

#[tokio::test]
async fn real_wss_untrusted_certificate_is_terminal() {
    tokio::time::timeout(Duration::from_secs(5), handshake(false, "localhost"))
        .await
        .unwrap();
}

#[tokio::test]
async fn real_wss_wrong_hostname_is_terminal() {
    tokio::time::timeout(Duration::from_secs(5), handshake(true, "wrong.example"))
        .await
        .unwrap();
}
