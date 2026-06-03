use super::PrinterInfo;
use crate::model::MediaSize;
use ipp::value::IppKeyword;
use std::collections::HashSet;
use std::hash::Hash;

fn distinct_values<T: Eq + Hash>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    values
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

#[derive(Debug, Clone)]
/// A struct to hold the supported values for media attributes, extracted from `PrinterInfo`.
pub struct MediaSupportedValues {
    pub names: Vec<IppKeyword>,
    pub sizes: Vec<MediaSize>,
    pub top_margins: Vec<i32>,
    pub right_margins: Vec<i32>,
    pub bottom_margins: Vec<i32>,
    pub left_margins: Vec<i32>,
    pub colors: Vec<IppKeyword>,
    pub sources: Vec<IppKeyword>,
    pub media_types: Vec<IppKeyword>,
    pub has_media_col_support: bool,
    pub media_col_support_set: HashSet<IppKeyword>,
}

impl MediaSupportedValues {
    pub fn new(info: &PrinterInfo) -> Self {
        Self {
            names: distinct_values(
                info.media_supported
                    .iter()
                    .filter_map(|media| media.name.clone()),
            ),
            sizes: distinct_values(info.media_supported.iter().filter_map(|media| media.size)),
            top_margins: distinct_values(
                info.media_supported
                    .iter()
                    .filter_map(|media| media.margins.map(|margins| margins.0)),
            ),
            right_margins: distinct_values(
                info.media_supported
                    .iter()
                    .filter_map(|media| media.margins.map(|margins| margins.1)),
            ),
            bottom_margins: distinct_values(
                info.media_supported
                    .iter()
                    .filter_map(|media| media.margins.map(|margins| margins.2)),
            ),
            left_margins: distinct_values(
                info.media_supported
                    .iter()
                    .filter_map(|media| media.margins.map(|margins| margins.3)),
            ),
            colors: distinct_values(
                info.media_supported
                    .iter()
                    .filter_map(|media| media.color.clone()),
            ),
            sources: distinct_values(
                info.media_supported
                    .iter()
                    .filter_map(|media| media.source.clone()),
            ),
            media_types: distinct_values(
                info.media_supported
                    .iter()
                    .filter_map(|media| media.media_type.clone()),
            ),
            has_media_col_support: !info.media_col_supported.is_empty(),
            media_col_support_set: info.media_col_supported.iter().cloned().collect(),
        }
    }
}
