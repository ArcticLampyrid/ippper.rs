#![cfg(all(
    feature = "server-tls",
    any(feature = "rustls-aws-lc-rs", feature = "rustls-ring")
))]

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use ippper::server::serve_adaptive_https;
use std::{future::Future, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::{
    rustls::{self, pki_types::PrivatePkcs8KeyDer, ClientConfig, RootCertStore, ServerConfig},
    TlsConnector,
};

async fn with_server<F: Future<Output = ()>>(
    version: &'static rustls::SupportedProtocolVersion,
    run: impl FnOnce(SocketAddr, TlsConnector) -> F,
) {
    #[cfg(feature = "rustls-aws-lc-rs")]
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    #[cfg(not(feature = "rustls-aws-lc-rs"))]
    let provider = rustls::crypto::ring::default_provider();
    let provider = Arc::new(provider);
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let server = ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        )
        .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let mut client = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[version])
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let service = service_fn(|request: Request<Incoming>| async {
        let (parts, body) = request.into_parts();
        let body = body.collect().await?.to_bytes();
        let echo = format!(
            "{} {}: {}",
            parts.method,
            parts.uri,
            String::from_utf8_lossy(&body)
        );
        Ok::<_, hyper::Error>(Response::new(Full::new(Bytes::from(echo))))
    });
    // The public API binds its own listener; release an ephemeral port for it.
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = reservation.local_addr().unwrap();
    drop(reservation);
    tokio::select! {
        biased;
        result = serve_adaptive_https(addr, service, Arc::new(server)) =>
            panic!("server stopped: {result:?}"),
        result = timeout(Duration::from_secs(5), run(addr, TlsConnector::from(Arc::new(client)))) =>
            result.expect("adaptive HTTPS timed out"),
    }
}

// Read exactly one response, leaving any following TLS or HTTP bytes untouched.
async fn response(stream: &mut (impl AsyncRead + Unpin)) -> (String, String) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(head).unwrap();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |length| length.parse().unwrap());
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
    (head, String::from_utf8(body).unwrap())
}

#[tokio::test]
async fn http_and_https_share_port() {
    with_server(&rustls::version::TLS13, |addr, tls| async move {
        let mut plain = TcpStream::connect(addr).await.unwrap();
        plain
            .write_all(b"GET /plain HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let (head, body) = response(&mut plain).await;
        assert!(head.starts_with("HTTP/1.1 200"));
        assert_eq!(body, "GET /plain: ");

        let mut secure = tls
            .connect(
                "localhost".try_into().unwrap(),
                TcpStream::connect(addr).await.unwrap(),
            )
            .await
            .unwrap();
        secure
            .write_all(b"GET /secure HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let (head, body) = response(&mut secure).await;
        assert!(head.starts_with("HTTP/1.1 200"));
        assert_eq!(body, "GET /secure: ");
    })
    .await;
}

#[tokio::test]
async fn unsupported_upgrade_stays_plaintext() {
    with_server(&rustls::version::TLS13, |addr, _| async move {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                b"GET / HTTP/1.1\r\nHost: localhost\r\n\
                  Connection: Upgrade\r\nUpgrade: TLS/1.0\r\n\r\n",
            )
            .await
            .unwrap();
        let (head, body) = response(&mut stream).await;
        assert!(head.starts_with("HTTP/1.1 200"));
        assert_eq!(body, "GET /: ");
    })
    .await;
}

#[tokio::test]
async fn upgrade_is_advisory_and_replays_request() {
    for (advertised, negotiated) in [
        ("TLS/1.3", &rustls::version::TLS12),
        ("TLS/1.2", &rustls::version::TLS13),
    ] {
        with_server(negotiated, |addr, tls| async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let request = format!(
                "POST /print HTTP/1.1\r\nHost: localhost\r\n\
                 Connection: Upgrade\r\nUpgrade: {advertised}\r\n\
                 Transfer-Encoding: chunked\r\nExpect: 100-continue\r\n\r\n"
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            assert!(response(&mut stream).await.0.starts_with("HTTP/1.1 100"));
            stream.write_all(b"3\r\njob\r\n0\r\n\r\n").await.unwrap();
            let (head, body) = response(&mut stream).await;
            assert!(head.starts_with("HTTP/1.1 101"));
            assert!(head.contains(&format!("upgrade: {advertised}, HTTP/1.1\r\n")));
            assert!(body.is_empty());

            let mut stream = tls
                .connect("localhost".try_into().unwrap(), stream)
                .await
                .unwrap();
            assert_eq!(
                stream.get_ref().1.protocol_version(),
                Some(negotiated.version)
            );
            assert_eq!(stream.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
            let (head, body) = response(&mut stream).await;
            assert!(head.starts_with("HTTP/1.1 200"));
            assert_eq!(body, "POST /print: job");

            stream
                .write_all(b"GET /next HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await
                .unwrap();
            let (head, body) = response(&mut stream).await;
            assert!(head.starts_with("HTTP/1.1 200"));
            assert_eq!(body, "GET /next: ");
        })
        .await;
    }
}
