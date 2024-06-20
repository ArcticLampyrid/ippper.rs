use crate::error::IppError;
use crate::model::{PageOrientation, Resolution};
use crate::result::IppResult;
use crate::service::IppService;
use crate::utils::{get_ipp_attribute, remove_ipp_attribute};
use anyhow;
use async_compression::futures::bufread;
use ipp::attribute::IppAttribute;
use ipp::model::{DelimiterTag, IppVersion, JobState, Operation, PrinterState, StatusCode};
use ipp::payload::IppPayload;
use ipp::request::IppRequestResponse;
use ipp::value::IppValue;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub trait SimpleIppServiceHandler: Send + Sync {
    fn handle_document(
        &self,
        _document: SimpleIppDocument,
    ) -> impl futures::Future<Output = anyhow::Result<()>> + Send {
        futures::future::ready(Ok(()))
    }
}

pub struct SimpleIppDocument {
    pub format: Option<String>,
    pub media: Option<String>,
    pub orientation: Option<PageOrientation>,
    pub sides: Option<String>,
    pub print_color_mode: Option<String>,
    pub printer_resolution: Option<Resolution>,

    pub payload: IppPayload,
}

#[derive(Debug, Clone, Builder)]
pub struct PrinterInfo {
    #[builder(default = r#""IppServer".to_string()"#)]
    name: String,
    #[builder(default = r#"Some("IppServer by ippper".to_string())"#)]
    info: Option<String>,
    #[builder(default = r#"Some("IppServer by ippper".to_string())"#)]
    make_and_model: Option<String>,
    #[builder(default = r#"None"#)]
    uuid: Option<Uuid>,
    #[builder(default = r#"vec!["application/pdf".to_string()]"#)]
    document_formats_supported: Vec<String>,
    #[builder(default = r#""application/pdf".to_string()"#)]
    document_format_default: String,
    #[builder(default = r#"Some("application/pdf".to_string())"#)]
    document_format_preferred: Option<String>,
    #[builder(default = r#"vec!["iso_a4_210x297mm".to_string()]"#)]
    media_supported: Vec<String>,
    #[builder(default = r#""iso_a4_210x297mm".to_string()"#)]
    media_default: String,
    #[builder(default = r#"vec![PageOrientation::Portrait]"#)]
    orientation_supported: Vec<PageOrientation>,
    #[builder(default = r#"None"#)]
    orientation_default: Option<PageOrientation>,
    #[builder(default = r#"vec!["one-sided".to_string()]"#)]
    side_supported: Vec<String>,
    #[builder(default = r#""one-sided".to_string()"#)]
    side_default: String,
    #[builder(default = r#"vec!["monochrome".to_string(), "color".to_string()]"#)]
    print_color_mode_supported: Vec<String>,
    #[builder(default = r#""monochrome".to_string()"#)]
    print_color_mode_default: String,
    #[builder(default = r#"None"#)]
    printer_resolution_supported: Option<Vec<Resolution>>,
    #[builder(default = r#"None"#)]
    printer_resolution_default: Option<Resolution>,
}

pub struct SimpleIppService<T: SimpleIppServiceHandler> {
    start_time: Instant,
    job_id: AtomicI32,
    host: String,
    info: PrinterInfo,
    handler: T,
}
impl<T: SimpleIppServiceHandler> SimpleIppService<T> {
    pub fn new(info: PrinterInfo, handler: T) -> Self {
        Self {
            start_time: Instant::now(),
            job_id: AtomicI32::new(1000),
            host: "defaulthost:631".to_string(),
            info,
            handler,
        }
    }
    pub fn set_host(&mut self, host: &str) {
        self.host = host.to_string();
    }
    pub fn set_info(&mut self, info: PrinterInfo) {
        self.info = info;
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
                IppValue::MimeMediaType(self.info.document_format_default.clone()),
            ),
            IppAttribute::new(
                IppAttribute::DOCUMENT_FORMAT_SUPPORTED,
                IppValue::Array(
                    self.info
                        .document_formats_supported
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
                IppValue::Keyword(self.info.media_default.clone()),
            ),
            IppAttribute::new(
                IppAttribute::MEDIA_SUPPORTED,
                IppValue::Array(
                    self.info
                        .media_supported
                        .iter()
                        .map(|media| IppValue::Keyword(media.clone()))
                        .collect::<Vec<_>>(),
                ),
            ),
            IppAttribute::new(
                IppAttribute::ORIENTATION_REQUESTED_DEFAULT,
                self.info
                    .orientation_default
                    .map(|orientation| orientation.into())
                    .unwrap_or(IppValue::NoValue),
            ),
            IppAttribute::new(
                IppAttribute::ORIENTATION_REQUESTED_SUPPORTED,
                IppValue::Array(
                    self.info
                        .orientation_supported
                        .iter()
                        .map(|orientation| (*orientation).into())
                        .collect::<Vec<_>>(),
                ),
            ),
            IppAttribute::new(
                IppAttribute::SIDES_DEFAULT,
                IppValue::Keyword(self.info.side_default.clone()),
            ),
            IppAttribute::new(
                IppAttribute::SIDES_SUPPORTED,
                IppValue::Array(
                    self.info
                        .side_supported
                        .iter()
                        .map(|side| IppValue::Keyword(side.clone()))
                        .collect::<Vec<_>>(),
                ),
            ),
            IppAttribute::new(
                IppAttribute::PRINT_COLOR_MODE_DEFAULT,
                IppValue::Keyword(self.info.print_color_mode_default.clone()),
            ),
            IppAttribute::new(
                IppAttribute::PRINT_COLOR_MODE_SUPPORTED,
                IppValue::Array(
                    self.info
                        .print_color_mode_supported
                        .iter()
                        .map(|mode| IppValue::Keyword(mode.clone()))
                        .collect::<Vec<_>>(),
                ),
            ),
        ];
        if let Some(preferred) = self.info.document_format_preferred.clone() {
            r.push(IppAttribute::new(
                "document-format-preferred",
                IppValue::MimeMediaType(preferred),
            ));
        }
        if let Some(ref supported) = self.info.printer_resolution_supported {
            r.push(IppAttribute::new(
                IppAttribute::PRINTER_RESOLUTION_SUPPORTED,
                IppValue::Array(
                    supported
                        .iter()
                        .map(|resolution| IppValue::from(*resolution))
                        .collect::<Vec<_>>(),
                ),
            ));
        }
        if let Some(default) = self.info.printer_resolution_default {
            r.push(IppAttribute::new(
                IppAttribute::PRINTER_RESOLUTION_DEFAULT,
                default.into(),
            ));
        }

        let mut job_creation_attributes_supported = vec![
            IppValue::Keyword("job-name".to_string()),
            IppValue::Keyword("media".to_string()),
            IppValue::Keyword("orientation-requested".to_string()),
            IppValue::Keyword("print-color-mode".to_string()),
            IppValue::Keyword("sides".to_string()),
        ];
        if self.info.printer_resolution_supported.is_some() {
            job_creation_attributes_supported
                .push(IppValue::Keyword("printer-resolution".to_string()));
        }
        r.push(IppAttribute::new(
            "job-creation-attributes-supported",
            IppValue::Array(job_creation_attributes_supported),
        ));

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

impl<T: SimpleIppServiceHandler> IppService for SimpleIppService<T> {
    fn version(&self) -> IppVersion {
        IppVersion::v2_0()
    }
    async fn print_job(&self, mut req: IppRequestResponse) -> IppResult {
        // Take the attributes from the request, leaving an empty set of attributes
        // in the request. This will avoid the need to clone the attributes.
        let mut attributes = std::mem::take(req.attributes_mut());

        let req_id = req.header().request_id;
        let version = req.header().version;

        let format = remove_ipp_attribute(
            &mut attributes,
            DelimiterTag::OperationAttributes,
            "document-format",
        )
        .and_then(|attr| attr.into_mime_media_type().ok());

        // Check if the requested document format is supported
        if let Some(ref x) = format {
            if !self.info.document_formats_supported.contains(x) {
                return Err(IppError {
                    code: StatusCode::ClientErrorDocumentFormatNotSupported,
                    msg: StatusCode::ClientErrorDocumentFormatNotSupported.to_string(),
                }
                .into());
            }
        }

        let media = remove_ipp_attribute(&mut attributes, DelimiterTag::JobAttributes, "media")
            .and_then(|attr| attr.into_keyword().ok());

        let orientation = remove_ipp_attribute(
            &mut attributes,
            DelimiterTag::JobAttributes,
            "orientation-requested",
        )
        .and_then(|attr| PageOrientation::try_from(attr).ok());

        let sides = remove_ipp_attribute(&mut attributes, DelimiterTag::JobAttributes, "sides")
            .and_then(|attr| attr.into_keyword().ok());

        let print_color_mode = remove_ipp_attribute(
            &mut attributes,
            DelimiterTag::JobAttributes,
            "print-color-mode",
        )
        .and_then(|attr| attr.into_keyword().ok());

        let printer_resolution = remove_ipp_attribute(
            &mut attributes,
            DelimiterTag::JobAttributes,
            "printer-resolution",
        )
        .and_then(|attr| Resolution::try_from(attr).ok());

        let compression = remove_ipp_attribute(
            &mut attributes,
            DelimiterTag::OperationAttributes,
            "compression",
        )
        .and_then(|attr| match attr {
            IppValue::Keyword(x) => {
                if x == "none" {
                    None
                } else {
                    Some(x)
                }
            }
            _ => None,
        });

        let payload = match compression.as_deref() {
            None => req.into_payload(),
            Some("gzip") => {
                let raw_payload = req.into_payload();
                let decoder = bufread::GzipDecoder::new(futures::io::BufReader::new(raw_payload));
                IppPayload::new_async(decoder)
            }
            _ => {
                return Err(IppError {
                    code: StatusCode::ClientErrorCompressionNotSupported,
                    msg: StatusCode::ClientErrorCompressionNotSupported.to_string(),
                }
                .into())
            }
        };
        self.handler
            .handle_document(SimpleIppDocument {
                format,
                media,
                orientation,
                sides,
                print_color_mode,
                printer_resolution,
                payload,
            })
            .await?;
        let mut resp = IppRequestResponse::new_response(version, StatusCode::SuccessfulOk, req_id);
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
        let job_id = get_ipp_attribute(
            req.attributes(),
            DelimiterTag::OperationAttributes,
            IppAttribute::JOB_ID,
        )
        .and_then(|attr| attr.as_integer());
        match job_id {
            Some(job_id) => {
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
        let optional_requested_attributes = get_ipp_attribute(
            req.attributes(),
            DelimiterTag::OperationAttributes,
            IppAttribute::REQUESTED_ATTRIBUTES,
        )
        .map(|attr| {
            attr.into_iter()
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
