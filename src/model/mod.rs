use ipp::value::IppValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageOrientation {
    Portrait = 3,
    Landscape,
    ReversePortrait,
    ReverseLandscape,
}

impl TryFrom<i32> for PageOrientation {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            3 => Ok(Self::Portrait),
            4 => Ok(Self::Landscape),
            5 => Ok(Self::ReversePortrait),
            6 => Ok(Self::ReverseLandscape),
            _ => Err(value),
        }
    }
}

impl From<PageOrientation> for i32 {
    fn from(value: PageOrientation) -> Self {
        value as i32
    }
}

impl TryFrom<IppValue> for PageOrientation {
    type Error = IppValue;

    fn try_from(value: IppValue) -> Result<Self, IppValue> {
        match value {
            IppValue::Enum(v) => Self::try_from(v).map_err(|_| IppValue::Enum(v)),
            _ => Err(value),
        }
    }
}

impl From<PageOrientation> for IppValue {
    fn from(value: PageOrientation) -> Self {
        IppValue::Enum(value as i32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Resolution {
    pub cross_feed: i32,
    pub feed: i32,
    pub units: i8,
}

impl Resolution {
    pub fn new_dpi(cross_feed: i32, feed: i32) -> Self {
        Self {
            cross_feed,
            feed,
            units: 3,
        }
    }
    pub fn new_dpcm(cross_feed: i32, feed: i32) -> Self {
        Self {
            cross_feed,
            feed,
            units: 4,
        }
    }
}

impl TryFrom<IppValue> for Resolution {
    type Error = IppValue;

    fn try_from(value: IppValue) -> Result<Self, IppValue> {
        if let IppValue::Resolution {
            cross_feed,
            feed,
            units,
        } = value
        {
            Ok(Self {
                cross_feed,
                feed,
                units,
            })
        } else {
            Err(value)
        }
    }
}

impl From<Resolution> for IppValue {
    fn from(value: Resolution) -> Self {
        IppValue::Resolution {
            cross_feed: value.cross_feed,
            feed: value.feed,
            units: value.units,
        }
    }
}
