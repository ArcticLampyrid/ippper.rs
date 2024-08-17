use crate::error::IppError;
use crate::model::{PageOrientation, Resolution, WhichJob};
use crate::result::IppResult;
use crate::service::IppService;
use crate::utils::{
    decommpress_payload, get_ipp_attribute, get_requested_attributes, take_ipp_attribute,
    take_requesting_user_name,
};
use anyhow;
use futures_locks::RwLock;
use http::request::Parts as ReqParts;
use http::uri::Scheme;
use ipp::attribute::{IppAttribute, IppAttributeGroup, IppAttributes};
use ipp::model::{DelimiterTag, IppVersion, JobState, Operation, PrinterState, StatusCode};
use ipp::payload::IppPayload;
use ipp::request::IppRequestResponse;
use ipp::value::IppValue;
use moka::future::{Cache, CacheBuilder};
use std::collections::HashSet;
use std::ops::Deref;
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
    pub originating_user_name: String,
    pub media: String,
    pub orientation: Option<PageOrientation>,
    pub sides: String,
    pub print_color_mode: String,
    pub printer_resolution: Option<Resolution>,
}

impl SimpleIppJobAttributes {
    pub(crate) fn take_ipp_attributes(
        info: &PrinterInfo,
        originating_user_name: String,
        attributes: &mut IppAttributes,
    ) -> Self {
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
            originating_user_name,
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
    #[builder(default = r#"vec![]"#)]
    urf_supported: Vec<String>,
    #[builder(default = r#"vec![]"#)]
    pwg_raster_document_type_supported: Vec<String>,
    #[builder(default = r#"vec![]"#)]
    pwg_raster_document_resolution_supported: Vec<Resolution>,
    #[builder(default = r#"None"#)]
    pwg_raster_document_sheet_back: Option<String>,
}

#[derive(Debug, Clone)]
struct JobInfo {
    id: i32,
    state: JobState,
    state_message: String,
    state_reasons: IppValue,
    attributes: SimpleIppJobAttributes,
    created_at: Duration,
    processing_at: Option<Duration>,
    completed_at: Option<Duration>,
}

pub struct SimpleIppService<T: SimpleIppServiceHandler> {
    start_time: Instant,
    job_id: AtomicI32,
    job_snapshot: Cache<i32, RwLock<JobInfo>>,
    host: String,
    basepath: String,
    info: PrinterInfo,
    handler: T,
}
impl<T: SimpleIppServiceHandler> SimpleIppService<T> {
    pub fn new(info: PrinterInfo, handler: T) -> Self {
        let job_snapshot = CacheBuilder::new(1000)
            .time_to_live(Duration::from_secs(60 * 15))
            .build();
        Self {
            start_time: Instant::now(),
            job_id: AtomicI32::new(1000),
            job_snapshot,
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
    fn printer_attributes(&self, head: &ReqParts, requested: &HashSet<&str>) -> Vec<IppAttribute> {
        let mut r = Vec::<IppAttribute>::new();
        let requested_all = requested.contains("all");

        macro_rules! is_requested {
            ($name:expr) => {
                requested_all || requested.contains($name)
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
            IppValue::Keyword("requesting-user-name".to_string())
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
                IppValue::Enum(Operation::CreateJob as i32),
                IppValue::Enum(Operation::SendDocument as i32),
                IppValue::Enum(Operation::CancelJob as i32),
                IppValue::Enum(Operation::GetJobAttributes as i32),
                IppValue::Enum(Operation::GetJobs as i32),
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
        if !self.info.urf_supported.is_empty() {
            add_if_requested!(
                "urf-supported",
                IppValue::Array(
                    self.info
                        .urf_supported
                        .clone()
                        .into_iter()
                        .map(IppValue::Keyword)
                        .collect::<Vec<_>>()
                )
            );
        }
        if !self.info.pwg_raster_document_type_supported.is_empty() {
            add_if_requested!(
                "pwg-raster-document-type-supported",
                IppValue::Array(
                    self.info
                        .pwg_raster_document_type_supported
                        .clone()
                        .into_iter()
                        .map(IppValue::Keyword)
                        .collect::<Vec<_>>()
                )
            );
        }
        if !self
            .info
            .pwg_raster_document_resolution_supported
            .is_empty()
        {
            add_if_requested!(
                "pwg-raster-document-resolution-supported",
                IppValue::Array(
                    self.info
                        .pwg_raster_document_resolution_supported
                        .clone()
                        .into_iter()
                        .map(IppValue::from)
                        .collect::<Vec<_>>()
                )
            );
        }
        optional_add_if_requested!(
            "pwg-raster-document-sheet-back",
            self.info
                .pwg_raster_document_sheet_back
                .clone()
                .map(IppValue::Keyword)
        );
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
    async fn alloc_job(&self, init: impl FnOnce(i32) -> JobInfo) -> RwLock<JobInfo> {
        let id = self.job_id.fetch_add(1, Ordering::Relaxed);
        let data = RwLock::new(init(id));
        self.job_snapshot.insert(id, data.clone()).await;
        data
    }
    async fn find_job(&self, r: &IppAttributes) -> anyhow::Result<RwLock<JobInfo>> {
        let job_id = get_ipp_attribute(r, DelimiterTag::OperationAttributes, IppAttribute::JOB_ID)
            .and_then(|attr| attr.as_integer())
            .cloned();
        let job = match job_id {
            Some(job_id) => self.job_snapshot.get(&job_id).await,
            _ => None,
        };
        match job {
            Some(job) => Ok(job),
            _ => Err(IppError {
                code: StatusCode::ClientErrorNotFound,
                msg: StatusCode::ClientErrorNotFound.to_string(),
            }
            .into()),
        }
    }
    fn take_document_format(&self, r: &mut IppAttributes) -> anyhow::Result<Option<String>> {
        let format = take_ipp_attribute(r, DelimiterTag::OperationAttributes, "document-format")
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

        Ok(format)
    }
    fn lite_job_attributes_for(&self, head: &ReqParts, job: &JobInfo) -> Vec<IppAttribute> {
        vec![
            IppAttribute::new(
                IppAttribute::JOB_URI,
                IppValue::Uri(self.make_url(head.uri.scheme(), format!("job/{}", job.id).as_str())),
            ),
            IppAttribute::new(IppAttribute::JOB_ID, IppValue::Integer(job.id)),
            IppAttribute::new(IppAttribute::JOB_STATE, IppValue::Enum(job.state as i32)),
            IppAttribute::new(
                "job-state-message",
                IppValue::TextWithoutLanguage(job.state_message.clone()),
            ),
            IppAttribute::new(IppAttribute::JOB_STATE_REASONS, job.state_reasons.clone()),
        ]
    }
    fn job_attributes_for(
        &self,
        head: &ReqParts,
        job: &JobInfo,
        requested: &HashSet<&str>,
    ) -> Vec<IppAttribute> {
        let mut r = Vec::<IppAttribute>::new();

        let requested_all = requested.contains("all");
        let requested_job_description = requested_all || requested.contains("job-description");
        let requested_job_template = requested_all || requested.contains("job-template");
        macro_rules! is_requested {
            (description : $name:expr) => {
                requested_job_description || requested.contains($name)
            };
            (template : $name:expr) => {
                requested_job_template || requested.contains($name)
            };
        }
        macro_rules! add_if_requested {
            ($kind:ident : $name:expr, $value:expr) => {
                if is_requested!($kind : $name) {
                    r.push(IppAttribute::new($name, $value));
                }
            };
        }
        macro_rules! optional_add_if_requested {
            ($kind:ident : $name:expr, $value:expr) => {
                if is_requested!($kind : $name) {
                    if let Some(value) = $value {
                        r.push(IppAttribute::new($name, value));
                    }
                }
            };
        }

        add_if_requested!(
            description: IppAttribute::JOB_URI,
            IppValue::Uri(self.make_url(head.uri.scheme(), format!("job/{}", job.id).as_str()))
        );
        add_if_requested!(description: IppAttribute::JOB_ID, IppValue::Integer(job.id));
        add_if_requested!(description: IppAttribute::JOB_STATE, IppValue::Enum(job.state as i32));
        add_if_requested!(description: "job-state-message", IppValue::TextWithoutLanguage(job.state_message.clone()));
        add_if_requested!(description: IppAttribute::JOB_STATE_REASONS, job.state_reasons.clone());
        add_if_requested!(
            description: "job-printer-uri",
            IppValue::Uri(self.make_url(head.uri.scheme(), "/"))
        );
        add_if_requested!(
            description: IppAttribute::JOB_NAME,
            IppValue::NameWithoutLanguage(format!("Job #{}", job.id))
        );
        add_if_requested!(
            description: "job-originating-user-name",
            IppValue::NameWithoutLanguage(job.attributes.originating_user_name.clone())
        );
        add_if_requested!(
            description: "time-at-creation",
            IppValue::Integer(job.created_at.as_secs() as i32)
        );
        add_if_requested!(
            description: "time-at-processing",
            job.processing_at
                .map_or(IppValue::NoValue, |x| IppValue::Integer(x.as_secs() as i32))
        );
        add_if_requested!(
            description: "time-at-completed",
            job.completed_at
                .map_or(IppValue::NoValue, |x| IppValue::Integer(x.as_secs() as i32))
        );
        add_if_requested!(
            description: "job-printer-up-time",
            IppValue::Integer(self.uptime().as_secs() as i32)
        );
        add_if_requested!(template: "media", IppValue::Keyword(job.attributes.media.clone()));
        add_if_requested!(
            template: "orientation-requested",
            job.attributes
                .orientation
                .map_or(IppValue::NoValue, IppValue::from)
        );
        add_if_requested!(template: "sides", IppValue::Keyword(job.attributes.sides.clone()));
        add_if_requested!(
            template: "print-color-mode",
            IppValue::Keyword(job.attributes.print_color_mode.clone())
        );
        optional_add_if_requested!(
            template: "printer-resolution",
            job.attributes.printer_resolution.map(IppValue::from)
        );
        r
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

        let requesting_user_name = take_requesting_user_name(&mut attributes);
        let job_attributes = SimpleIppJobAttributes::take_ipp_attributes(
            &self.info,
            requesting_user_name,
            &mut attributes,
        );

        let created_at = self.uptime();
        let job = self
            .alloc_job(|id| JobInfo {
                id,
                state: JobState::Processing,
                state_message: "Processing".to_string(),
                state_reasons: IppValue::Keyword("none".to_string()),
                attributes: job_attributes.clone(),
                created_at,
                processing_at: Some(created_at),
                completed_at: None,
            })
            .await;

        let format = self.take_document_format(&mut attributes)?;
        let compression = take_ipp_attribute(
            &mut attributes,
            DelimiterTag::OperationAttributes,
            "compression",
        )
        .and_then(|attr| attr.into_keyword().ok());
        let payload = decommpress_payload(req.into_payload(), compression.as_deref())?;
        let document_handled = self
            .handler
            .handle_document(SimpleIppDocument {
                format,
                job_attributes,
                payload,
            })
            .await;
        {
            let mut job = job.write().await;
            if let Err(ref error) = document_handled {
                job.state = JobState::Aborted;
                job.state_message = format!("Aborted: {}", error);
            } else {
                job.state = JobState::Completed;
                job.state_message = "Completed".to_string();
            };
            job.completed_at = Some(self.uptime());
        }

        let mut resp = if let Err(error) = document_handled {
            self.build_error_response(version, req_id, error)
        } else {
            IppRequestResponse::new_response(version, StatusCode::SuccessfulOk, req_id)
        };
        self.add_basic_attributes(&mut resp);
        let job_attributes = self.lite_job_attributes_for(&head, job.read().await.deref());
        let mut group = IppAttributeGroup::new(DelimiterTag::JobAttributes);
        group
            .attributes_mut()
            .extend(job_attributes.into_iter().map(|x| (x.name().to_owned(), x)));
        resp.attributes_mut().groups_mut().push(group);
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

    async fn create_job(&self, head: ReqParts, mut req: IppRequestResponse) -> IppResult {
        // Take the attributes from the request, leaving an empty set of attributes
        // in the request. This will avoid the need to clone the attributes.
        let mut attributes = std::mem::take(req.attributes_mut());

        let req_id = req.header().request_id;
        let version = req.header().version;

        let requesting_user_name = take_requesting_user_name(&mut attributes);
        let job_attributes = SimpleIppJobAttributes::take_ipp_attributes(
            &self.info,
            requesting_user_name,
            &mut attributes,
        );

        let created_at = self.uptime();
        let job = self
            .alloc_job(|id| JobInfo {
                id,
                state: JobState::Pending,
                state_message: "Pending".to_string(),
                state_reasons: IppValue::Keyword("none".to_string()),
                attributes: job_attributes.clone(),
                created_at,
                processing_at: Some(created_at),
                completed_at: None,
            })
            .await;

        let mut resp = IppRequestResponse::new_response(version, StatusCode::SuccessfulOk, req_id);
        self.add_basic_attributes(&mut resp);
        let job_attributes = self.lite_job_attributes_for(&head, job.read().await.deref());
        let mut group = IppAttributeGroup::new(DelimiterTag::JobAttributes);
        group
            .attributes_mut()
            .extend(job_attributes.into_iter().map(|x| (x.name().to_owned(), x)));
        resp.attributes_mut().groups_mut().push(group);
        Ok(resp)
    }

    async fn send_document(&self, head: ReqParts, mut req: IppRequestResponse) -> IppResult {
        let req_id = req.header().request_id;
        let version = req.header().version;

        let job = self.find_job(req.attributes()).await?;

        // Update the job state to processing
        let job_attributes;
        {
            let mut job = job.write().await;
            if job.state != JobState::Processing {
                job.state = JobState::Processing;
                job.state_message = "Processing".to_string();
                job.processing_at = Some(self.uptime());
            }
            job_attributes = job.attributes.clone();
        }

        // Take the attributes from the request, leaving an empty set of attributes
        // in the request. This will avoid the need to clone the attributes.
        let mut attributes = std::mem::take(req.attributes_mut());

        let format = self.take_document_format(&mut attributes)?;
        let compression = take_ipp_attribute(
            &mut attributes,
            DelimiterTag::OperationAttributes,
            "compression",
        )
        .and_then(|attr| attr.into_keyword().ok());
        let payload = decommpress_payload(req.into_payload(), compression.as_deref())?;
        let document_handled = self
            .handler
            .handle_document(SimpleIppDocument {
                format,
                job_attributes,
                payload,
            })
            .await;
        {
            let mut job = job.write().await;
            if let Err(ref error) = document_handled {
                job.state = JobState::Aborted;
                job.state_message = format!("Aborted: {}", error);
            } else {
                job.state = JobState::Completed;
                job.state_message = "Completed".to_string();
            };
            job.completed_at = Some(self.uptime());
        }

        let mut resp = if let Err(error) = document_handled {
            self.build_error_response(version, req_id, error)
        } else {
            IppRequestResponse::new_response(version, StatusCode::SuccessfulOk, req_id)
        };
        self.add_basic_attributes(&mut resp);
        let job_attributes = self.lite_job_attributes_for(&head, job.read().await.deref());
        let mut group = IppAttributeGroup::new(DelimiterTag::JobAttributes);
        group
            .attributes_mut()
            .extend(job_attributes.into_iter().map(|x| (x.name().to_owned(), x)));
        resp.attributes_mut().groups_mut().push(group);
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
        let job = self.find_job(req.attributes()).await?;
        let requested_attributes = get_requested_attributes(req.attributes());
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);
        let job_attributes =
            self.job_attributes_for(&head, job.read().await.deref(), &requested_attributes);
        let mut group = IppAttributeGroup::new(DelimiterTag::JobAttributes);
        group
            .attributes_mut()
            .extend(job_attributes.into_iter().map(|x| (x.name().to_owned(), x)));
        resp.attributes_mut().groups_mut().push(group);
        Ok(resp)
    }

    async fn get_jobs(&self, head: ReqParts, mut req: IppRequestResponse) -> IppResult {
        let mut count = 0;
        let limit = take_ipp_attribute(
            req.attributes_mut(),
            DelimiterTag::OperationAttributes,
            "limit",
        )
        .and_then(|attr| attr.into_integer().ok());

        let which_jobs = take_ipp_attribute(
            req.attributes_mut(),
            DelimiterTag::OperationAttributes,
            "which-jobs",
        )
        .and_then(|attr| attr.into_keyword().ok());

        let which_jobs = match which_jobs.as_deref() {
            Some("completed") => WhichJob::Completed,
            Some("not-completed") => WhichJob::NotCompleted,
            _ => WhichJob::NotCompleted,
        };

        let requested_attributes = get_requested_attributes(req.attributes());

        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);

        for (_, job) in self.job_snapshot.iter() {
            let job = job.read().await;
            if which_jobs == WhichJob::from(job.state) {
                let job_attributes =
                    self.job_attributes_for(&head, job.deref(), &requested_attributes);
                let mut group = IppAttributeGroup::new(DelimiterTag::JobAttributes);
                group
                    .attributes_mut()
                    .extend(job_attributes.into_iter().map(|x| (x.name().to_owned(), x)));
                resp.attributes_mut().groups_mut().push(group);

                count += 1;
                if limit.map_or(false, |x| count >= x) {
                    break;
                }
            }
        }

        Ok(resp)
    }

    async fn get_printer_attributes(&self, head: ReqParts, req: IppRequestResponse) -> IppResult {
        let mut resp = IppRequestResponse::new_response(
            req.header().version,
            StatusCode::SuccessfulOk,
            req.header().request_id,
        );
        self.add_basic_attributes(&mut resp);
        let requested_attributes = get_requested_attributes(req.attributes());
        let printer_attributes = self.printer_attributes(&head, &requested_attributes);
        let mut group = IppAttributeGroup::new(DelimiterTag::PrinterAttributes);
        group.attributes_mut().extend(
            printer_attributes
                .into_iter()
                .map(|x| (x.name().to_owned(), x)),
        );
        resp.attributes_mut().groups_mut().push(group);
        Ok(resp)
    }
}
