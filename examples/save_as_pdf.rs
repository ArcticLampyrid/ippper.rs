use async_trait::async_trait;
use ipp::payload::IppPayload;
use ippper::server::IppServer;
use ippper::service::{PrinterInfoBuilder, SimpleIppService, SimpleIppServiceHandler};
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
#[async_trait]
impl SimpleIppServiceHandler for MyHandler {
    async fn handle_document(
        &self,
        _document_format: &str,
        payload: &mut IppPayload,
    ) -> anyhow::Result<()> {
        let mut file = File::create("D:\\1.pdf").await?;
        io::copy(&mut payload.compat(), &mut file).await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 631);
    let mut ipp_handler = SimpleIppService::new(MyHandler::new());
    ipp_handler.set_info(
        PrinterInfoBuilder::default()
            .uuid(Some(
                // Change it if you are building a ipp service
                // Make it unique for each instance
                Uuid::parse_str("786a551c-65a3-43ce-89ba-33c51bae9bc2").unwrap(),
            ))
            .build()
            .unwrap(),
    );
    if let Err(e) = IppServer::serve(addr, Arc::new(ipp_handler)).await {
        eprintln!("server error: {}", e);
    }
}
