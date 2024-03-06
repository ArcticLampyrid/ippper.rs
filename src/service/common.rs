use crate::error::IppError;
use crate::result::IppResult;
use anyhow;
use async_trait::async_trait;
use ipp::attribute::IppAttribute;
use ipp::model::{DelimiterTag, IppVersion, Operation, StatusCode};
use ipp::request::IppRequestResponse;
use ipp::value::IppValue;
use num_traits::FromPrimitive;

fn operation_not_supported() -> anyhow::Error {
    anyhow::Error::new(IppError {
        code: StatusCode::ServerErrorOperationNotSupported,
        msg: StatusCode::ServerErrorOperationNotSupported.to_string(),
    })
}

#[async_trait]
pub trait IppService: Send + Sync + 'static {
    async fn print_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn print_uri(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn validate_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn create_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn send_document(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn send_uri(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn cancel_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn get_job_attributes(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn get_jobs(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn get_printer_attributes(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn hold_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn release_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn restart_job(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn pause_printer(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn resume_printer(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    async fn purge_jobs(&self, _req: IppRequestResponse) -> IppResult {
        Err(operation_not_supported())
    }

    fn version(&self) -> IppVersion {
        IppVersion::v1_1()
    }

    fn check_version(&self, req: &IppRequestResponse) -> bool {
        let version = req.header().version.0;
        version <= self.version().0
    }

    fn build_error_response(
        &self,
        version: IppVersion,
        req_id: u32,
        error: anyhow::Error,
    ) -> IppRequestResponse {
        let ipp_error = match error.downcast_ref::<IppError>() {
            Some(e) => e.clone(),
            None => IppError {
                code: StatusCode::ServerErrorInternalError,
                msg: error.to_string(),
            },
        };
        let mut resp = IppRequestResponse::new_response(version, ipp_error.code, req_id);
        resp.attributes_mut().add(
            DelimiterTag::OperationAttributes,
            IppAttribute::new(
                IppAttribute::STATUS_MESSAGE,
                IppValue::TextWithoutLanguage(ipp_error.msg),
            ),
        );
        resp
    }

    async fn handle_request(&self, req: IppRequestResponse) -> IppRequestResponse {
        let req_id = req.header().request_id;
        if !self.check_version(&req) {
            return self.build_error_response(
                self.version(),
                req_id,
                IppError {
                    code: StatusCode::ServerErrorVersionNotSupported,
                    msg: StatusCode::ServerErrorVersionNotSupported.to_string(),
                }
                .into(),
            );
        }
        let version = req.header().version;
        match Operation::from_u16(req.header().operation_or_status) {
            Some(op) => match op {
                Operation::PrintJob => self.print_job(req).await,
                Operation::PrintUri => self.print_uri(req).await,
                Operation::ValidateJob => self.validate_job(req).await,
                Operation::CreateJob => self.create_job(req).await,
                Operation::SendDocument => self.send_document(req).await,
                Operation::SendUri => self.send_uri(req).await,
                Operation::CancelJob => self.cancel_job(req).await,
                Operation::GetJobAttributes => self.get_job_attributes(req).await,
                Operation::GetJobs => self.get_jobs(req).await,
                Operation::GetPrinterAttributes => self.get_printer_attributes(req).await,
                Operation::HoldJob => self.hold_job(req).await,
                Operation::ReleaseJob => self.release_job(req).await,
                Operation::RestartJob => self.restart_job(req).await,
                Operation::PausePrinter => self.pause_printer(req).await,
                Operation::ResumePrinter => self.resume_printer(req).await,
                Operation::PurgeJobs => self.purge_jobs(req).await,
                _ => Err(operation_not_supported()),
            },
            None => Err(operation_not_supported()),
        }
        .unwrap_or_else(|error| self.build_error_response(version, req_id, error))
    }
}
