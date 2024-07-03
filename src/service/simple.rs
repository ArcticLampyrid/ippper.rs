use crate::error::IppError;
use crate::model::{PageOrientation, Resolution};
use crate::result::IppResult;
use crate::service::IppService;
use crate::utils::{get_ipp_attribute, take_ipp_attribute};
use anyhow;
use async_compression::futures::bufread;
use http::request::Parts as ReqParts;
use http::uri::Scheme;
use ipp::attribute::{IppAttribute, IppAttributes};
use ipp::model::{DelimiterTag, IppVersion, JobState, Operation, PrinterState, StatusCode};
use ipp::payload::IppPayload;
use ipp::request::IppRequestResponse;
use ipp::value::IppValue;
use std::collections::HashSet;
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

#[derive(fmt_derive::Debug)]
pub struct SimpleIppDocument {
    pub format: Option<String>,
    pub job_attributes: SimpleIppJobAttributes,

    #[fmt(ignore)]
    pub payload: IppPayload,
}

#[derive(fmt_derive::Debug, Clone)]
pub struct SimpleIppJobAttributes {
    pub media: String,
    pub orientation: Option<PageOrientation>,
    pub sides: String,
    pub print_color_mode: String,
    pub printer_resolution: Option<Resolution>,
}

impl SimpleIppJobAttributes {
    pub(crate) fn take_ipp_attributes(info: &PrinterInfo, attributes: &mut IppAttributes) -> Self {
        let media = take_ipp_attribute(attributes, DelimiterTag::JobAttributes, "media")
            .and_then(|attr| attr.into_keyword().ok())
            .unwrap_or_else(|| info.media_default.clone());

        let orientation = take_ipp_attribute(
            attributes,
            DelimiterTag::JobAttributes,
            "orientation-requested",
        )
        .and_then(|attr| PageOrientation::try_from(attr).ok())
        .or(info.orientation_default);

        let sides = take_ipp_attribute(attributes, DelimiterTag::JobAttributes, "sides")
            .and_then(|attr| attr.into_keyword().ok())
            .unwrap_or_else(|| info.sides_default.clone());

        let print_color_mode =
            take_ipp_attribute(attributes, DelimiterTag::JobAttributes, "print-color-mode")
                .and_then(|attr| attr.into_keyword().ok())
                .unwrap_or_else(|| info.print_color_mode_default.clone());

        let printer_resolution = take_ipp_attribute(
            attributes,
            DelimiterTag::JobAttributes,
            "printer-resolution",
        )
        .and_then(|attr| Resolution::try_from(attr).ok())
        .or(info.printer_resolution_default);
        Self {
            media,
            orientation,
            sides,
            print_color_mode,
            printer_resolution,
        }
    }
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
    document_format_supported: Vec<String>,
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
    sides_supported: Vec<String>,
    #[builder(default = r#""one-sided".to_string()"#)]
    sides_default: String,
    #[builder(default = r#"vec!["monochrome".to_string(), "color".to_string()]"#)]
    print_color_mode_supported: Vec<String>,
    #[builder(default = r#""monochrome".to_string()"#)]
    print_color_mode_default: String,
    #[builder(default = r#"vec![]"#)]
    printer_resolution_supported: Vec<Resolution>,
    #[builder(default = r#"None"#)]
    printer_resolution_default: Option<Resolution>,
    #[builder(default = r#"vec![
        "adobe-1.2".to_string(),
        "adobe-1.3".to_string(),
        "adobe-1.4".to_string(),
        "adobe-1.5".to_string(),
        "adobe-1.6".to_string(),
        "adobe-1.7".to_string(),
        "iso-19005-1_2005".to_string(),
        "iso-32000-1_2008".to_string(),
        "pwg-5102.3".to_string(),
    ]"#)]
    pdf_versions_supported: Vec<String>,
}

pub struct SimpleIppService<T: SimpleIppServiceHandler> {
    start_time: Instant,
    job_id: AtomicI32,
    host: String,
    basepath: String,
    info: PrinterInfo,
    handler: T,
}
impl<T: SimpleIppServiceHandler> SimpleIppService<T> {
    pub fn new(info: PrinterInfo, handler: T) -> Self {
        Self {
            start_time: Instant::now(),
            job_id: AtomicI32::new(1000),
            host: "defaulthost:631".to_string(),
            basepath: "/".to_string(),
            info,
            handler,
        }
    }
    pub fn set_host(&mut self, host: &str) {
        self.host = host.to_string();
    }
    pub fn set_basepath(&mut self, basepath: &str) {
        self.basepath = basepath.to_string();
    }
    pub fn set_info(&mut self, info: PrinterInfo) {
        self.info = info;
    }
    fn make_url(&self, scheme: Option<&Scheme>, path: &str) -> String {
        let basepath = self.basepath.trim_start_matches('/').trim_end_matches('/');
        let slash_before_basepath = if basepath.is_empty() { "" } else { "/" };
        let slash_before_path = if path.starts_with('/') { "" } else { "/" };
        let scheme = scheme.map_or("ipp", |x| x.as_str());
        format!(
            "{}://{}{}{}{}{}",
            scheme, self.host, slash_before_basepath, basepath, slash_before_path, path
        )
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
    fn printer_attributes(
        &self,
        head: &ReqParts,
        requested: Option<&HashSet<&str>>,
    ) -> Vec<IppAttribute> {
        let mut r = Vec::<IppAttribute>::new();

        macro_rules! is_requested {
            ($name:expr) => {
                requested.map_or(true, |x| x.contains($name))
            };
        }
        macro_rules! add_if_requested {
            ($name:expr, $value:expr) => {
                if is_requested!($name) {
                    r.push(IppAttribute::new($name, $value));
                }
            };
        }
        macro_rules! optional_add_if_requested {
            ($name:expr, $value:expr) => {
                if is_requested!($name) {
                    if let Some(value) = $value {
                        r.push(IppAttribute::new($name, value));
                    }
                }
            };
        }

        add_if_requested!(
            IppAttribute::PRINTER_URI_SUPPORTED,
            IppValue::Uri(self.make_url(head.uri.scheme(), "/"))
        );
        add_if_requested!(
            IppAttribute::URI_AUTHENTICATION_SUPPORTED,
            IppValue::Keyword("none".to_string())
        );
        add_if_requested!(
            IppAttribute::URI_SECURITY_SUPPORTED,
            IppValue::Keyword(
                match head.uri.scheme_str() {
                    Some("ipps") => "tls",
                    Some("https") => "tls",
                    _ => "none",
                }
                .to_string()
            )
        );
        add_if_requested!(
            IppAttribute::PRINTER_NAME,
            IppValue::NameWithoutLanguage(self.info.name.clone())
        );
        add_if_requested!(
            IppAttribute::PRINTER_STATE,
            IppValue::Enum(PrinterState::Idle as i32)
        );
        add_if_requested!(
            IppAttribute::PRINTER_STATE_REASONS,
            IppValue::Keyword("none".to_string())
        );
        add_if_requested!(
            IppAttribute::IPP_VERSIONS_SUPPORTED,
            IppValue::Array(vec![
                IppValue::Keyword("1.0".to_string()),
                IppValue::Keyword("1.1".to_string()),
                IppValue::Keyword("2.0".to_string()),
            ])
        );
        add_if_requested!(
            IppAttribute::OPERATIONS_SUPPORTED,
            IppValue::Array(vec![
                IppValue::Enum(Operation::PrintJob as i32),
                IppValue::Enum(Operation::ValidateJob as i32),
                IppValue::Enum(Operation::CancelJob as i32),
                IppValue::Enum(Operation::GetJobAttributes as i32),
                IppValue::Enum(Operation::GetPrinterAttributes as i32),
            ])
        );
        add_if_requested!("multiple-document-jobs-supported", IppValue::Boolean(false));
        add_if_requested!(
            IppAttribute::CHARSET_CONFIGURED,
            IppValue::Charset("utf-8".to_string())
        );
        add_if_requested!(
            IppAttribute::CHARSET_SUPPORTED,
            IppValue::Charset("utf-8".to_string())
        );
        add_if_requested!(
            IppAttribute::NATURAL_LANGUAGE_CONFIGURED,
            IppValue::NaturalLanguage("en".to_string())
        );
        add_if_requested!(
            IppAttribute::GENERATED_NATURAL_LANGUAGE_SUPPORTED,
            IppValue::NaturalLanguage("en".to_string())
        );
        add_if_requested!(
            IppAttribute::DOCUMENT_FORMAT_DEFAULT,
            IppValue::MimeMediaType(self.info.document_format_default.clone())
        );
        add_if_requested!(
            IppAttribute::DOCUMENT_FORMAT_SUPPORTED,
            IppValue::Array(
                self.info
                    .document_format_supported
                    .clone()
                    .into_iter()
                    .map(IppValue::MimeMediaType)
                    .collect::<Vec<_>>()
            )
        );
        add_if_requested!(
            IppAttribute::PRINTER_IS_ACCEPTING_JOBS,
            IppValue::Boolean(true)
        );
        add_if_requested!(
            IppAttribute::PDL_OVERRIDE_SUPPORTED,
            IppValue::Keyword("attempted".to_string())
        );
        add_if_requested!(
            IppAttribute::PRINTER_UP_TIME,
            IppValue::Integer(self.uptime().as_secs() as i32)
        );
        add_if_requested!(
            IppAttribute::COMPRESSION_SUPPORTED,
            IppValue::Array(vec![
                IppValue::Keyword("none".to_string()),
                IppValue::Keyword("gzip".to_string()),
            ])
        );
        add_if_requested!(
            IppAttribute::MEDIA_DEFAULT,
            IppValue::Keyword(self.info.media_default.clone())
        );
        add_if_requested!(
            IppAttribute::MEDIA_SUPPORTED,
            IppValue::Array(
                self.info
                    .media_supported
                    .clone()
                    .into_iter()
                    .map(IppValue::Keyword)
                    .collect::<Vec<_>>()
            )
        );
        add_if_requested!(
            IppAttribute::ORIENTATION_REQUESTED_DEFAULT,
            self.info
                .orientation_default
                .map(|orientation| orientation.into())
                .unwrap_or(IppValue::NoValue)
        );
        add_if_requested!(
            IppAttribute::ORIENTATION_REQUESTED_SUPPORTED,
            IppValue::Array(
                self.info
                    .orientation_supported
                    .clone()
                    .into_iter()
                    .map(IppValue::from)
                    .collect::<Vec<_>>()
            )
        );
        add_if_requested!(
            IppAttribute::SIDES_DEFAULT,
            IppValue::Keyword(self.info.sides_default.clone())
        );
        add_if_requested!(
            IppAttribute::SIDES_SUPPORTED,
            IppValue::Array(
                self.info
                    .sides_supported
                    .clone()
                    .into_iter()
                    .map(IppValue::Keyword)
                    .collect::<Vec<_>>()
            )
        );
        add_if_requested!(
            IppAttribute::PRINT_COLOR_MODE_DEFAULT,
            IppValue::Keyword(self.info.print_color_mode_default.clone())
        );
        add_if_requested!(
            IppAttribute::PRINT_COLOR_MODE_SUPPORTED,
            IppValue::Array(
                self.info
                    .print_color_mode_supported
                    .clone()
                    .into_iter()
                    .map(IppValue::Keyword)
                    .collect::<Vec<_>>()
            )
        );
        optional_add_if_requested!(
            "document-format-preferred",
            self.info
                .document_format_preferred
                .clone()
                .map(IppValue::MimeMediaType)
        );
        if !self.info.printer_resolution_supported.is_empty() {
            add_if_requested!(
                IppAttribute::PRINTER_RESOLUTION_SUPPORTED,
                IppValue::Array(
                    self.info
                        .printer_resolution_supported
                        .clone()
                        .into_iter()
                        .map(IppValue::from)
                        .collect::<Vec<_>>()
                )
            );
        }
        optional_add_if_requested!(
            IppAttribute::PRINTER_RESOLUTION_DEFAULT,
            self.info.printer_resolution_default.map(IppValue::from)
        );
        if !self.info.pdf_versions_supported.is_empty() {
            add_if_requested!(
                "pdf-versions-supported",
                IppValue::Array(
                    self.info
                        .pdf_versions_supported
                        .clone()
                        .into_iter()
                        .map(IppValue::Keyword)
                        .collect::<Vec<_>>()
                )
            );
        }
        if is_requested!("job-creation-attributes-supported") {
            let mut job_creation_attributes_supported = vec![
                IppValue::Keyword("job-name".to_string()),
                IppValue::Keyword("media".to_string()),
                IppValue::Keyword("orientation-requested".to_string()),
                IppValue::Keyword("print-color-mode".to_string()),
                IppValue::Keyword("sides".to_string()),
            ];
            if !self.info.printer_resolution_supported.is_empty() {
                job_creation_attributes_supported
                    .push(IppValue::Keyword("printer-resolution".to_string()));
            }
            r.push(IppAttribute::new(
                "job-creation-attributes-supported",
                IppValue::Array(job_creation_attributes_supported),
            ));
        }
        optional_add_if_requested!(
            IppAttribute::PRINTER_INFO,
            self.info.info.clone().map(IppValue::TextWithoutLanguage)
        );
        optional_add_if_requested!(
            IppAttribute::PRINTER_MAKE_AND_MODEL,
            self.info
                .make_and_model
                .clone()
                .map(IppValue::TextWithoutLanguage)
        );
        optional_add_if_requested!(
            "printer-uuid",
            self.info.uuid.map(|uuid| IppValue::Uri(
                uuid.urn()
                    .encode_lower(&mut Uuid::encode_buffer())
                    .to_string()
            ))
        );

        r
    }
    fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }
    fn job_attributes(
        &self,
        head: &ReqParts,
        job_id: i32,
        job_state: JobState,
        job_state_reasons: IppValue,
    ) -> Vec<IppAttribute> {
        vec![
            IppAttribute::new(
                IppAttribute::JOB_URI,
                IppValue::Uri(self.make_url(head.uri.scheme(), format!("job/{}", job_id).as_str())),
            ),
            IppAttribute::new(IppAttribute::JOB_ID, IppValue::Integer(job_id)),
            IppAttribute::new(IppAttribute::JOB_STATE, IppValue::Enum(job_state as i32)),
            IppAttribute::new(IppAttribute::JOB_STATE_REASONS, job_state_reasons),
            IppAttribute::new(
                "job-printer-uri",
                IppValue::Uri(self.make_url(head.uri.scheme(), "/")),
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
    async fn print_job(&self, head: ReqParts, mut req: IppRequestResponse) -> IppResult {
        // Take the attributes from the request, leaving an empty set of attributes
        // in the request. This will avoid the need to clone the attributes.
        let mut attributes = std::mem::take(req.attributes_mut());

        let req_id = req.header().request_id;
        let version = req.header().version;

        let format = take_ipp_attribute(
            &mut attributes,
            DelimiterTag::OperationAttributes,
            "document-format",
        )
        .and_then(|attr| attr.into_mime_media_type().ok());

        // Check if the requested document format is supported
        if let Some(ref x) = format {
            if !self.info.document_format_supported.contains(x) {
                return Err(IppError {
                    code: StatusCode::ClientErrorDocumentFormatNotSupported,
                    msg: StatusCode::ClientErrorDocumentFormatNotSupported.to_string(),
                }
                .into());
            }
        }

        let job_attributes =
            SimpleIppJobAttributes::take_ipp_attributes(&self.info, &mut attributes);

        let compression = take_ipp_attribute(
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
                job_attributes,
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
            &head,
            job_id,
            JobState::Completed,
            IppValue::Keyword("none".to_string()),
        );
        for attr in job_attributes {
            resp.attributes_mut().add(DelimiterTag::JobAttributes, attr);
        }
        Ok(resp)
    }

    async fn validate_job(&self, _head: ReqParts, req: IppRequestResponse) -> IppResult {
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);
        Ok(resp)
    }

    async fn cancel_job(&self, _head: ReqParts, req: IppRequestResponse) -> IppResult {
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);
        Ok(resp)
    }

    async fn get_job_attributes(&self, head: ReqParts, req: IppRequestResponse) -> IppResult {
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
                    &head,
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

    async fn get_printer_attributes(&self, head: ReqParts, req: IppRequestResponse) -> IppResult {
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);
        let requested_attributes = get_ipp_attribute(
            req.attributes(),
            DelimiterTag::OperationAttributes,
            IppAttribute::REQUESTED_ATTRIBUTES,
        )
        .map(|attr| {
            attr.into_iter()
                .filter_map(|e| e.as_keyword().map(|x| x.as_str()))
                .collect::<HashSet<_>>()
        })
        .filter(|x| !x.contains("all"));
        let printer_attributes = self.printer_attributes(&head, requested_attributes.as_ref());
        for attr in printer_attributes {
            resp.attributes_mut()
                .add(DelimiterTag::PrinterAttributes, attr);
        }
        Ok(resp)
    }
}
