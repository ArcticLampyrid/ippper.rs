use ippper::model::{PageOrientation, Resolution};
use ippper::server::{serve_adaptive_https, tls_config_from_reader, wrap_as_http_service};
use ippper::service::simple::{
    PrinterInfoBuilder, SimpleIppDocument, SimpleIppService, SimpleIppServiceHandler,
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io;
use tokio_util::compat::*;
use uuid::Uuid;

struct MyHandler {}
impl MyHandler {
    fn new() -> Self {
        Self {}
    }
}

impl SimpleIppServiceHandler for MyHandler {
    fn handle_document(
        &self,
        document: SimpleIppDocument,
    ) -> impl futures::Future<Output = anyhow::Result<()>> + Send {
        async move {
            println!("Received document: {:#?}", document);
            let mut file = File::create("D:\\1.pdf").await?;
            io::copy(&mut document.payload.compat(), &mut file).await?;
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 631);
    let info = PrinterInfoBuilder::default()
        .uuid(Some(
            // Change it if you are building a ipp service
            // Make it unique for each instance
            Uuid::parse_str("786a551c-65a3-43ce-89ba-33c51bae9bc2").unwrap(),
        ))
        .sides_supported(vec![
            "one-sided".to_string(),
            "two-sided-long-edge".to_string(),
            "two-sided-short-edge".to_string(),
        ])
        .printer_resolution_supported(vec![
            Resolution {
                cross_feed: 300,
                feed: 300,
                units: 3,
            },
            Resolution {
                cross_feed: 600,
                feed: 600,
                units: 3,
            },
        ])
        .printer_resolution_default(Some(Resolution {
            cross_feed: 600,
            feed: 600,
            units: 3,
        }))
        .orientation_supported(vec![PageOrientation::Portrait, PageOrientation::Landscape])
        .build()
        .unwrap();
    let ipp_service = Arc::new(SimpleIppService::new(info, MyHandler::new()));
    const CERTIFICATE: &[u8] = include_bytes!("certificate/certificate.pem");
    const KEY: &[u8] = include_bytes!("certificate/key.pem");
    let tls_config =
        Arc::new(tls_config_from_reader(CERTIFICATE, KEY).expect("Failed to load tls config"));
    serve_adaptive_https(addr, wrap_as_http_service(ipp_service), tls_config).await
}
