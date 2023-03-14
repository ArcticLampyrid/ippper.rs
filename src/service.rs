use crate::error::IppError;
use crate::result::IppResult;
use crate::server::IppServerHandler;
use anyhow;
use async_compression::futures::bufread;
use async_trait::async_trait;
use ipp::attribute::IppAttribute;
use ipp::model::{DelimiterTag, IppVersion, JobState, Operation, PrinterState, StatusCode};
use ipp::payload::IppPayload;
use ipp::request::IppRequestResponse;
use ipp::value::IppValue;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[async_trait]
pub trait SimpleIppServiceHandler: Send + Sync + 'static {
    async fn handle_document(
        &self,
        _document: SimpleIppDocument,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct SimpleIppDocument {
    pub format: Option<String>,
    pub payload: IppPayload
}

#[derive(Debug, Clone, Builder)]
pub struct PrinterInfo {
    #[builder(default = r#""IppServer".to_string()"#)]
    pub name: String,
    #[builder(default = r#"Some("IppServer by ippper".to_string())"#)]
    pub info: Option<String>,
    #[builder(default = r#"Some("IppServer by ippper".to_string())"#)]
    pub make_and_model: Option<String>,
    #[builder(default = r#"None"#)]
    pub uuid: Option<Uuid>,
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
            info: PrinterInfoBuilder::default().build().unwrap(),
            handler,
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
                IppValue::Array(vec![
                    IppValue::Keyword("1.0".to_string()),
                    IppValue::Keyword("1.1".to_string()),
                    IppValue::Keyword("2.0".to_string()),
                ]),
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
                IppValue::Keyword("attempted".to_string()),
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
            IppAttribute::new(
                IppAttribute::MEDIA_DEFAULT,
                IppValue::Keyword("iso_a4_210x297mm".to_string()),
            ),
            IppAttribute::new(
                IppAttribute::MEDIA_SUPPORTED,
                IppValue::Array(
                    vec![
                        "na_letter_8.5x11in".to_string(),
                        "na_legal_8.5x14in".to_string(),
                        "na_executive_7.25x10.5in".to_string(),
                        "na_ledger_11x17in".to_string(),
                        "iso_a3_297x420mm".to_string(),
                        "iso_a4_210x297mm".to_string(),
                        "iso_a5_148x210mm".to_string(),
                        "jis_b5_182x257mm".to_string(),
                        "iso_b5_176x250mm".to_string(),
                        "na_number-10_4.125x9.5in".to_string(),
                        "iso_c5_162x229mm".to_string(),
                        "iso_dl_110x220mm".to_string(),
                        "na_monarch_3.875x7.5in".to_string(),
                    ]
                    .iter()
                    .map(|format| IppValue::Keyword(format.clone()))
                    .collect::<Vec<_>>(),
                ),
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
        if let Some(uuid) = self.info.uuid {
            r.push(IppAttribute::new(
                "printer-uuid",
                IppValue::Uri(
                    uuid.urn()
                        .encode_lower(&mut Uuid::encode_buffer())
                        .to_string(),
                ),
            ))
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
    fn version(&self) -> IppVersion {
        IppVersion::v2_0()
    }
    async fn print_job(&self, req: IppRequestResponse) -> IppResult {
        let format = req
            .attributes()
            .groups_of(DelimiterTag::OperationAttributes)
            .next()
            .and_then(|g| g.attributes().get(IppAttribute::JOB_ID))
            .map(|attr| attr.value())
            .and_then(|attr| match attr {
                IppValue::MimeMediaType(x) => Some(x.clone()),
                _ => None,
            });
            
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
        let version = req.header().version;
        if let Some(ref x) = format {
            if !self.supported_document_formats.contains(x) {
                return Err(IppError {
                    code: StatusCode::ClientErrorDocumentFormatNotSupported,
                    msg: StatusCode::ClientErrorDocumentFormatNotSupported.to_string(),
                }
                .into());
            }
        }
        match compression {
            None => {
                self.handler
                    .handle_document(SimpleIppDocument{
                        format, 
                        payload: req.into_payload()
                    })
                    .await?
            }
            Some("gzip") => {
                let raw_payload = req.into_payload();
                let decoder = bufread::GzipDecoder::new(futures::io::BufReader::new(raw_payload));
                let payload = IppPayload::new_async(decoder);
                self.handler
                    .handle_document(SimpleIppDocument{
                        format, 
                        payload
                    })
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
            version,
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
