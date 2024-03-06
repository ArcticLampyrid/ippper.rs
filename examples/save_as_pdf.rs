use async_trait::async_trait;
use ippper::server::serve_ipp;
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
#[async_trait]
impl SimpleIppServiceHandler for MyHandler {
    async fn handle_document(&self, document: SimpleIppDocument) -> anyhow::Result<()> {
        let mut file = File::create("D:\\1.pdf").await?;
        io::copy(&mut document.payload.compat(), &mut file).await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 631);
    let mut ipp_service = SimpleIppService::new(MyHandler::new());
    ipp_service.set_info(
        PrinterInfoBuilder::default()
            .uuid(Some(
                // Change it if you are building a ipp service
                // Make it unique for each instance
                Uuid::parse_str("786a551c-65a3-43ce-89ba-33c51bae9bc2").unwrap(),
            ))
            .build()
            .unwrap(),
    );
    serve_ipp(addr, Arc::new(ipp_service)).await
}
