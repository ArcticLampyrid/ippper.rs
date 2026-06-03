use ipp::value::{IppKeyword, IppName, IppValue};
use std::collections::BTreeMap;

use super::media_size::MediaSize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaInfo {
    /// PWG 5101.1 media name, also used for legacy `media`.
    pub name: Option<IppKeyword>,

    /// Media dimensions in hundredths of millimeters.
    pub size: Option<MediaSize>,

    /// `(top, right, bottom, left)` in hundredths of millimeters.
    pub margins: Option<(i32, i32, i32, i32)>,

    /// IPP `media-color` keyword.
    pub color: Option<IppKeyword>,

    /// IPP `media-source` keyword.
    pub source: Option<IppKeyword>,

    /// IPP `media-type` keyword.
    pub media_type: Option<IppKeyword>,
}

impl MediaInfo {
    pub fn name_only(name: IppKeyword) -> Self {
        Self {
            name: Some(name),
            size: None,
            margins: None,
            color: None,
            source: None,
            media_type: None,
        }
    }
}

impl From<BTreeMap<IppName, IppValue>> for MediaInfo {
    fn from(collection: BTreeMap<IppName, IppValue>) -> Self {
        let size = collection
            .get("media-size")
            .and_then(|value| value.as_collection())
            .map(MediaSize::from);

        let top = collection
            .get("media-top-margin")
            .and_then(|value| value.as_integer().copied());
        let right = collection
            .get("media-right-margin")
            .and_then(|value| value.as_integer().copied());
        let bottom = collection
            .get("media-bottom-margin")
            .and_then(|value| value.as_integer().copied());
        let left = collection
            .get("media-left-margin")
            .and_then(|value| value.as_integer().copied());
        let margins = if top.is_some() || right.is_some() || bottom.is_some() || left.is_some() {
            Some((
                top.unwrap_or(0),
                right.unwrap_or(0),
                bottom.unwrap_or(0),
                left.unwrap_or(0),
            ))
        } else {
            None
        };

        Self {
            name: collection
                .get("media-size-name")
                .and_then(|value| value.as_keyword().cloned()),
            size,
            margins,
            color: collection
                .get("media-color")
                .and_then(|value| value.as_keyword().cloned()),
            source: collection
                .get("media-source")
                .and_then(|value| value.as_keyword().cloned()),
            media_type: collection
                .get("media-type")
                .and_then(|value| value.as_keyword().cloned()),
        }
    }
}

impl From<&MediaInfo> for IppValue {
    fn from(media: &MediaInfo) -> Self {
        let mut collection: BTreeMap<IppName, IppValue> = BTreeMap::new();

        if let Some(size) = media.size {
            collection.insert("media-size".try_into().unwrap(), IppValue::from(size));
        }

        if let Some(name) = &media.name {
            collection.insert(
                "media-size-name".try_into().unwrap(),
                IppValue::Keyword(name.clone()),
            );
        }

        if let Some((top, right, bottom, left)) = media.margins {
            collection.insert(
                "media-top-margin".try_into().unwrap(),
                IppValue::Integer(top),
            );
            collection.insert(
                "media-right-margin".try_into().unwrap(),
                IppValue::Integer(right),
            );

            collection.insert(
                "media-bottom-margin".try_into().unwrap(),
                IppValue::Integer(bottom),
            );

            collection.insert(
                "media-left-margin".try_into().unwrap(),
                IppValue::Integer(left),
            );
        }

        if let Some(color) = &media.color {
            collection.insert(
                "media-color".try_into().unwrap(),
                IppValue::Keyword(color.clone()),
            );
        }

        if let Some(source) = &media.source {
            collection.insert(
                "media-source".try_into().unwrap(),
                IppValue::Keyword(source.clone()),
            );
        }

        if let Some(media_type) = &media.media_type {
            collection.insert(
                "media-type".try_into().unwrap(),
                IppValue::Keyword(media_type.clone()),
            );
        }

        IppValue::Collection(collection)
    }
}

impl From<MediaInfo> for IppValue {
    fn from(media: MediaInfo) -> Self {
        Self::from(&media)
    }
}
