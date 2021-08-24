use crate::error::IppError;
use crate::result::IppResult;
use crate::server::IppServerHandler;
use anyhow;
use async_compression::futures::bufread;
use async_trait::async_trait;
use ipp::attribute::IppAttribute;
use ipp::model::{DelimiterTag, JobState, Operation, PrinterState, StatusCode};
use ipp::payload::IppPayload;
use ipp::request::IppRequestResponse;
use ipp::value::IppValue;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

#[async_trait]
pub trait SimpleIppServiceHandler: Send + Sync + 'static {
    async fn handle_document(
        &self,
        _document_format: &str,
        _payload: &mut IppPayload,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PrinterInfo {
    pub name: String,
    pub info: Option<String>,
    pub make_and_model: Option<String>,
}
pub struct SimpleIppService<T: SimpleIppServiceHandler> {
    start_time: Instant,
    job_id: AtomicI32,
    host: String,
    info: PrinterInfo,
    default_document_format: String,
    supported_document_formats: Vec<String>,
    handler: T,
}
impl<T: SimpleIppServiceHandler> SimpleIppService<T> {
    pub fn new(handler: T) -> Self {
        Self {
            start_time: Instant::now(),
            job_id: AtomicI32::new(1000),
            host: "defaulthost:631".to_string(),
            info: PrinterInfo {
                name: "IppServer".to_string(),
                info: Some("IppServer by ippper".to_string()),
                make_and_model: Some("IppServer by ippper".to_string()),
            },
            handler: handler,
            default_document_format: "application/pdf".to_string(),
            supported_document_formats: vec!["application/pdf".to_string()],
        }
    }
    pub fn set_host(&mut self, host: &str) {
        self.host = host.to_string();
    }
    pub fn set_info(&mut self, info: PrinterInfo) {
        self.info = info;
    }
    pub fn set_document_format(
        &mut self,
        supported_document_formats: Vec<String>,
        default_document_format: String,
    ) {
        self.supported_document_formats = supported_document_formats;
        self.default_document_format = default_document_format;
        assert!(
            self.supported_document_formats
                .contains(&self.default_document_format),
            "default document format is out of supported document formats"
        );
    }
    fn add_basic_attributes(&self, resp: &mut IppRequestResponse) {
        resp.attributes_mut().add(
            DelimiterTag::OperationAttributes,
            IppAttribute::new(
                IppAttribute::ATTRIBUTES_CHARSET,
                IppValue::Charset("utf-8".to_string()),
            ),
        );
        resp.attributes_mut().add(
            DelimiterTag::OperationAttributes,
            IppAttribute::new(
                IppAttribute::ATTRIBUTES_NATURAL_LANGUAGE,
                IppValue::NaturalLanguage("en".to_string()),
            ),
        );
    }
    fn printer_attributes(&self) -> Vec<IppAttribute> {
        let mut r = vec![
            IppAttribute::new(
                IppAttribute::PRINTER_URI_SUPPORTED,
                IppValue::Uri(format!("ipp://{}/printer", self.host)),
            ),
            IppAttribute::new(
                IppAttribute::URI_AUTHENTICATION_SUPPORTED,
                IppValue::Keyword("none".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::URI_SECURITY_SUPPORTED,
                IppValue::Keyword("none".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::PRINTER_NAME,
                IppValue::NameWithoutLanguage(self.info.name.clone()),
            ),
            IppAttribute::new(
                IppAttribute::PRINTER_STATE,
                IppValue::Enum(PrinterState::Idle as i32),
            ),
            IppAttribute::new(
                IppAttribute::PRINTER_STATE_REASONS,
                IppValue::Keyword("none".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::IPP_VERSIONS_SUPPORTED,
                IppValue::Keyword("1.1".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::OPERATIONS_SUPPORTED,
                IppValue::Array(vec![
                    IppValue::Enum(Operation::PrintJob as i32),
                    IppValue::Enum(Operation::ValidateJob as i32),
                    IppValue::Enum(Operation::CancelJob as i32),
                    IppValue::Enum(Operation::GetJobAttributes as i32),
                    IppValue::Enum(Operation::GetPrinterAttributes as i32),
                ]),
            ),
            IppAttribute::new("multiple-document-jobs-supported", IppValue::Boolean(false)),
            IppAttribute::new(
                IppAttribute::CHARSET_CONFIGURED,
                IppValue::Charset("utf-8".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::CHARSET_SUPPORTED,
                IppValue::Charset("utf-8".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::NATURAL_LANGUAGE_CONFIGURED,
                IppValue::NaturalLanguage("en".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::GENERATED_NATURAL_LANGUAGE_SUPPORTED,
                IppValue::NaturalLanguage("en".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::DOCUMENT_FORMAT_DEFAULT,
                IppValue::MimeMediaType(self.default_document_format.clone()),
            ),
            IppAttribute::new(
                IppAttribute::DOCUMENT_FORMAT_SUPPORTED,
                IppValue::Array(
                    self.supported_document_formats
                        .iter()
                        .map(|format| IppValue::MimeMediaType(format.clone()))
                        .collect::<Vec<_>>(),
                ),
            ),
            IppAttribute::new(
                IppAttribute::PRINTER_IS_ACCEPTING_JOBS,
                IppValue::Boolean(true),
            ),
            IppAttribute::new(
                IppAttribute::PDL_OVERRIDE_SUPPORTED,
                IppValue::Keyword("not-attempted".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::PRINTER_UP_TIME,
                IppValue::Integer(self.uptime().as_secs() as i32),
            ),
            IppAttribute::new(
                IppAttribute::COMPRESSION_SUPPORTED,
                IppValue::Array(vec![
                    IppValue::Keyword("none".to_string()),
                    IppValue::Keyword("gzip".to_string()),
                ]),
            ),
        ];
        if let Some(info) = self.info.info.clone() {
            r.push(IppAttribute::new(
                IppAttribute::PRINTER_INFO,
                IppValue::TextWithoutLanguage(info),
            ));
        }
        if let Some(make_and_model) = self.info.make_and_model.clone() {
            r.push(IppAttribute::new(
                IppAttribute::PRINTER_MAKE_AND_MODEL,
                IppValue::TextWithoutLanguage(make_and_model),
            ));
        }
        r
    }
    fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }
    fn job_attributes(
        &self,
        job_id: i32,
        job_state: JobState,
        job_state_reasons: IppValue,
    ) -> Vec<IppAttribute> {
        vec![
            IppAttribute::new(
                IppAttribute::JOB_URI,
                IppValue::Uri(format!("ipp://{}/job/{}", self.host, job_id)),
            ),
            IppAttribute::new(IppAttribute::JOB_ID, IppValue::Integer(job_id)),
            IppAttribute::new(IppAttribute::JOB_STATE, IppValue::Enum(job_state as i32)),
            IppAttribute::new(IppAttribute::JOB_STATE_REASONS, job_state_reasons),
            IppAttribute::new(
                "job-printer-uri",
                IppValue::Uri(format!("ipp://{}/printer", self.host)),
            ),
            IppAttribute::new(
                IppAttribute::JOB_NAME,
                IppValue::TextWithoutLanguage("Print job ".to_string() + &job_id.to_string()),
            ),
            IppAttribute::new(
                "job-originating-user-name",
                IppValue::TextWithoutLanguage("IppSharing".to_string()),
            ),
            IppAttribute::new("time-at-creation", IppValue::Integer(0)),
            IppAttribute::new("time-at-processing", IppValue::Integer(0)),
            IppAttribute::new("time-at-completed", IppValue::Integer(0)),
            IppAttribute::new(
                IppAttribute::PRINTER_UP_TIME,
                IppValue::Integer(self.uptime().as_secs() as i32),
            ),
        ]
    }
}

#[async_trait]
impl<T: SimpleIppServiceHandler> IppServerHandler for SimpleIppService<T> {
    async fn print_job(&self, mut req: IppRequestResponse) -> IppResult {
        let document_format_value = req
            .attributes()
            .groups_of(DelimiterTag::OperationAttributes)
            .next()
            .and_then(|g| g.attributes().get(IppAttribute::JOB_ID))
            .map(|attr| attr.value());
        let document_format = match document_format_value {
            Some(IppValue::MimeMediaType(x)) => x.clone(),
            _ => self.default_document_format.clone(),
        };
        let compression = req
            .attributes()
            .groups_of(DelimiterTag::OperationAttributes)
            .next()
            .and_then(|g| g.attributes().get("compression"))
            .and_then(|attr| match attr.value() {
                IppValue::Keyword(x) => match x.as_ref() {
                    "none" => None,
                    x_ref => Some(x_ref),
                },
                _ => None,
            });
        let req_id = req.header().request_id;
        if !self.supported_document_formats.contains(&document_format) {
            return Err(IppError {
                code: StatusCode::ClientErrorDocumentFormatNotSupported,
                msg: StatusCode::ClientErrorDocumentFormatNotSupported.to_string(),
            }
            .into());
        }
        match compression {
            None => {
                self.handler
                    .handle_document(document_format.as_ref(), req.payload_mut())
                    .await?
            }
            Some("gzip") => {
                let raw_payload = std::mem::replace(req.payload_mut(), IppPayload::empty());
                let decoder = bufread::GzipDecoder::new(futures::io::BufReader::new(raw_payload));
                let mut payload = IppPayload::new_async(decoder);
                self.handler
                    .handle_document(document_format.as_ref(), &mut payload)
                    .await?
            }
            _ => {
                return Err(IppError {
                    code: StatusCode::ClientErrorCompressionNotSupported,
                    msg: StatusCode::ClientErrorCompressionNotSupported.to_string(),
                }
                .into())
            }
        }
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req_id,
        );
        self.add_basic_attributes(&mut resp);
        resp.attributes_mut().add(
            DelimiterTag::JobAttributes,
            IppAttribute::new(IppAttribute::JOB_ID, IppValue::Integer(1)),
        );
        let job_id = self.job_id.fetch_add(1, Ordering::Relaxed);
        let job_attributes = self.job_attributes(
            job_id,
            JobState::Processing,
            IppValue::Keyword("completed-successfully".to_string()),
        );
        for attr in job_attributes {
            resp.attributes_mut().add(DelimiterTag::JobAttributes, attr);
        }
        Ok(resp)
    }

    async fn validate_job(&self, req: IppRequestResponse) -> IppResult {
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);
        Ok(resp)
    }

    async fn cancel_job(&self, req: IppRequestResponse) -> IppResult {
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);
        Ok(resp)
    }

    async fn get_job_attributes(&self, req: IppRequestResponse) -> IppResult {
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        let job_id_value = req
            .attributes()
            .groups_of(DelimiterTag::OperationAttributes)
            .next()
            .and_then(|g| g.attributes().get(IppAttribute::JOB_ID))
            .map(|attr| attr.value());
        match job_id_value {
            Some(IppValue::Integer(job_id)) => {
                self.add_basic_attributes(&mut resp);
                let job_attributes = self.job_attributes(
                    *job_id,
                    JobState::Completed,
                    IppValue::Keyword("none".to_string()),
                );
                for attr in job_attributes {
                    resp.attributes_mut().add(DelimiterTag::JobAttributes, attr);
                }
                Ok(resp)
            }
            _ => Err(anyhow::Error::msg("failed to get job id")),
        }
    }

    async fn get_printer_attributes(&self, req: IppRequestResponse) -> IppResult {
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);
        let optional_requested_attributes = req
            .attributes()
            .groups_of(DelimiterTag::OperationAttributes)
            .next()
            .and_then(|g| g.attributes().get(IppAttribute::REQUESTED_ATTRIBUTES))
            .map(|attr| {
                attr.value()
                    .into_iter()
                    .filter_map(|e| e.as_keyword())
                    .map(|e| e.as_ref())
                    .collect::<Vec<_>>()
            });
        let printer_attributes = optional_requested_attributes.map_or(
            self.printer_attributes(),
            |requested_attributes| {
                self.printer_attributes()
                    .into_iter()
                    .filter(|attr| requested_attributes.contains(&attr.name()))
                    .collect::<Vec<_>>()
            },
        );
        for attr in printer_attributes {
            resp.attributes_mut()
                .add(DelimiterTag::PrinterAttributes, attr);
        }
        Ok(resp)
    }
}
