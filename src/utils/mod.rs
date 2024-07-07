use async_compression::futures::bufread;
use ipp::{
    attribute::{IppAttribute, IppAttributes},
    model::{DelimiterTag, StatusCode},
    payload::IppPayload,
    value::IppValue,
};
use std::collections::HashSet;
mod reader_stream;
use crate::error::IppError;
pub(crate) use reader_stream::ReaderStream;

pub fn get_ipp_attribute<'a>(
    r: &'a IppAttributes,
    tag: DelimiterTag,
    name: &str,
) -> Option<&'a IppValue> {
    r.groups_of(tag)
        .find_map(|g| g.attributes().get(name))
        .map(|a| a.value())
}

pub fn take_ipp_attribute(
    r: &mut IppAttributes,
    tag: DelimiterTag,
    name: &str,
) -> Option<IppValue> {
    r.groups_mut()
        .iter_mut()
        .filter(|g| g.tag() == tag)
        .find_map(|g| g.attributes_mut().remove(name))
        .map(|a| a.into_value())
}

pub fn decommpress_payload(
    payload: IppPayload,
    compression: Option<&str>,
) -> anyhow::Result<IppPayload> {
    match compression {
        None => Ok(payload),
        Some("none") => Ok(payload),
        Some("gzip") => {
            let decoder = bufread::GzipDecoder::new(futures::io::BufReader::new(payload));
            Ok(IppPayload::new_async(decoder))
        }
        _ => Err(IppError {
            code: StatusCode::ClientErrorCompressionNotSupported,
            msg: StatusCode::ClientErrorCompressionNotSupported.to_string(),
        }
        .into()),
    }
}

pub fn get_requested_attributes(r: &IppAttributes) -> HashSet<&str> {
    get_ipp_attribute(
        r,
        DelimiterTag::OperationAttributes,
        IppAttribute::REQUESTED_ATTRIBUTES,
    )
    .map(|attr| {
        attr.into_iter()
            .filter_map(|e| e.as_keyword().map(|x| x.as_str()))
            .collect::<HashSet<_>>()
    })
    .unwrap_or_else(|| HashSet::from(["all"]))
}

pub fn take_requesting_user_name(r: &mut IppAttributes) -> String {
    take_ipp_attribute(r, DelimiterTag::OperationAttributes, "requesting-user-name")
        .and_then(|attr| match attr {
            IppValue::NameWithoutLanguage(name) => Some(name),
            IppValue::NameWithLanguage { name, .. } => Some(name),
            _ => None,
        })
        .unwrap_or_else(|| "anonymous".to_string())
}
