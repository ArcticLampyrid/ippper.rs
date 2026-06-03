use ipp::value::{IppName, IppValue};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaSize {
    /// `x-dimension` in hundredths of millimeters.
    pub x_dimension: i32,

    /// `y-dimension` in hundredths of millimeters.
    pub y_dimension: i32,
}

impl MediaSize {
    pub fn new(x_dimension: i32, y_dimension: i32) -> Self {
        Self {
            x_dimension,
            y_dimension,
        }
    }
}

impl From<&BTreeMap<IppName, IppValue>> for MediaSize {
    fn from(collection: &BTreeMap<IppName, IppValue>) -> Self {
        Self {
            x_dimension: collection
                .get("x-dimension")
                .and_then(|value| value.as_integer().copied())
                .unwrap_or(0),
            y_dimension: collection
                .get("y-dimension")
                .and_then(|value| value.as_integer().copied())
                .unwrap_or(0),
        }
    }
}

impl From<BTreeMap<IppName, IppValue>> for MediaSize {
    fn from(collection: BTreeMap<IppName, IppValue>) -> Self {
        Self::from(&collection)
    }
}

impl From<&MediaSize> for IppValue {
    fn from(size: &MediaSize) -> Self {
        let mut collection: BTreeMap<IppName, IppValue> = BTreeMap::new();
        collection.insert(
            "x-dimension".try_into().unwrap(),
            IppValue::Integer(size.x_dimension),
        );
        collection.insert(
            "y-dimension".try_into().unwrap(),
            IppValue::Integer(size.y_dimension),
        );
        IppValue::Collection(collection)
    }
}

impl From<MediaSize> for IppValue {
    fn from(size: MediaSize) -> Self {
        Self::from(&size)
    }
}

impl fmt::Debug for MediaSize {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MediaSize({}.{:0>3} cm × {}.{:0>3} cm)",
            self.x_dimension / 1000,
            self.x_dimension % 1000,
            self.y_dimension / 1000,
            self.y_dimension % 1000
        )
    }
}
